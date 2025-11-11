#![allow(missing_docs)]
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::get,
    Router,
};
use serde::Serialize;
use std::sync::Arc;
use subxt::{client::OnlineClient, lightclient::LightClient, PolkadotConfig};
use tokio::sync::RwLock;

mod db;

#[subxt::subxt(runtime_metadata_path = "configs/polkadot_metadata_small.scale")]
pub mod polkadot {}

const POLKADOT_SPEC: &str = include_str!("../configs/polkadot.json");

// Configuration: Events to exclude (add pallets/methods here to save space)
const EXCLUDED_EVENTS: &[(&str, Option<&str>)] = &[
    // Example filters (uncomment to use):
    // ("System", Some("ExtrinsicSuccess")),  // Exclude System::ExtrinsicSuccess
    // ("Balances", None),                     // Exclude all Balances events
    // ("ParaInclusion", None),                // Exclude all ParaInclusion events (very verbose on relay chains)
];

// Configuration: Extrinsic actions to exclude (Pallet/Method format)
const EXCLUDED_EXTRINSICS: &[&str] = &[
    // Example filters (uncomment to use):
    // "Timestamp/set",           // Exclude timestamp extrinsics
    "ParaInherent/enter",      // Exclude para inherent extrinsics
];

#[derive(Clone, Serialize)]
struct EventInfo {
    pallet: String,
    variant: String,
    data: String,
}

#[derive(Clone, Serialize)]
struct ExtrinsicInfo {
    index: u32,
    hash: String,
    action: String,
    params: String,
    events: Vec<EventInfo>,
}

#[derive(Clone, Serialize)]
struct BlockInfo {
    number: u32,
    hash: String,
    extrinsics_count: usize,
    events_count: usize,
    extrinsics: Vec<ExtrinsicInfo>,
}

type SharedBlockInfo = Arc<RwLock<BlockInfo>>;

#[derive(Clone)]
struct AppState {
    block_info: SharedBlockInfo,
    db: Arc<db::Database>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    println!("Connecting to Polkadot via light client...\n");

    // Initialize database
    let event_filters: Vec<db::EventFilter> = EXCLUDED_EVENTS
        .iter()
        .map(|(pallet, method)| db::EventFilter {
            pallet: pallet.to_string(),
            method: method.map(|s| s.to_string()),
        })
        .collect();

    let extrinsic_filters: Vec<String> = EXCLUDED_EXTRINSICS
        .iter()
        .map(|s| s.to_string())
        .collect();

    let database = Arc::new(db::Database::new("./blocks.db", event_filters, extrinsic_filters)?);
    println!("Database initialized at ./blocks.db");

    if let Ok(Some(latest)) = database.get_latest_block_number() {
        println!("Latest block in database: #{}\n", latest);
    }

    let (_lightclient, polkadot_rpc) = LightClient::relay_chain(POLKADOT_SPEC)?;
    let polkadot_api = OnlineClient::<PolkadotConfig>::from_rpc_client(polkadot_rpc).await?;

    let block_info = Arc::new(RwLock::new(BlockInfo {
        number: 0,
        hash: String::from("0x0"),
        extrinsics_count: 0,
        events_count: 0,
        extrinsics: vec![],
    }));

    // Spawn block subscription task, this could use some cleaning up (not too much though!)
    let block_info_clone = block_info.clone();
    let db_clone = database.clone();
    tokio::spawn(async move {
        let mut blocks_sub = polkadot_api.blocks().subscribe_finalized().await.unwrap(); // double and triple check if this really gives the finalized stuff 
        while let Some(block) = blocks_sub.next().await {
            if let Ok(block) = block {
                let block_number = block.number();

                // Skip if block already exists in database (deduplication)
                if let Ok(Some(_)) = db_clone.get_block(block_number) {
                    continue;
                }

                let extrinsics = block.extrinsics().await.unwrap();
                let mut total_events = 0;

                let mut extrinsics_info: Vec<ExtrinsicInfo> = Vec::new();

                for extrinsic_details in extrinsics.iter() {
                    let idx = extrinsic_details.index();
                    let hash = format!("{:?}", extrinsic_details.hash());
                    let meta = extrinsic_details.extrinsic_metadata().ok();
                    let action = meta
                        .map(|m| format!("{}/{}", m.pallet.name(), m.variant.name))
                        .unwrap_or_else(|| "unknown".to_string());

                    // Apply extrinsic filtering
                    if !db_clone.should_include_extrinsic(&action) {
                        continue;
                    }

                    // Get extrinsic parameters
                    let params = extrinsic_details
                        .field_values()
                        .ok()
                        .map(|fv| format!("{}", fv))
                        .unwrap_or_else(|| "".to_string());

                    // Get events for this extrinsic
                    let events = extrinsic_details.events().await.unwrap();
                    let mut events_info: Vec<EventInfo> = Vec::new();

                    for evt in events.iter() {
                        if let Ok(evt) = evt {
                            let pallet = evt.pallet_name();
                            let variant = evt.variant_name();

                            // Apply filtering
                            if !db_clone.should_include_event(pallet, variant) {
                                continue;
                            }

                            let field_values = evt.field_values().ok();
                            events_info.push(EventInfo {
                                pallet: pallet.to_string(),
                                variant: variant.to_string(),
                                data: field_values
                                    .map(|fv| format!("{}", fv))
                                    .unwrap_or_else(|| "".to_string()),
                            });
                        }
                    }

                    total_events += events_info.len();

                    extrinsics_info.push(ExtrinsicInfo {
                        index: idx,
                        hash,
                        action,
                        params,
                        events: events_info,
                    });
                }

                let block_number = block.number();
                let block_hash = format!("{:?}", block.hash());

                // Update in-memory state
                let mut info = block_info_clone.write().await;
                info.number = block_number;
                info.hash = block_hash.clone();
                info.extrinsics_count = extrinsics_info.len();
                info.events_count = total_events;
                info.extrinsics = extrinsics_info.clone();

                // Store in database
                let stored_block = db::StoredBlock {
                    number: block_number,
                    hash: block_hash.clone(),
                    extrinsics: extrinsics_info.iter().map(|e| serde_json::to_value(e).unwrap()).collect(),
                    timestamp: chrono::Utc::now().timestamp(),
                };

                if let Err(e) = db_clone.store_block(&stored_block) {
                    eprintln!("Failed to store block #{}: {}", block_number, e);
                }

                println!("Block #{} - {} extrinsics, {} events (stored)",
                    info.number, info.extrinsics_count, info.events_count);
            }
        }
    });

    // Build API
    let app_state = AppState {
        block_info,
        db: database,
    };

    let app = Router::new()
        .route("/blocks/head", get(get_head_block))
        .route("/block/:number", get(get_block_by_number))
        .with_state(app_state);

    println!("\nSmolcar API running on http://localhost:8080");
    println!("Endpoints:");
    println!("  - http://localhost:8080/blocks/head");
    println!("  - http://localhost:8080/block/{{number}}\n");

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn get_head_block(State(state): State<AppState>) -> Json<BlockInfo> {
    let info = state.block_info.read().await;
    Json(info.clone())
}

async fn get_block_by_number(
    State(state): State<AppState>,
    Path(block_number): Path<u32>,
) -> impl IntoResponse {
    match state.db.get_block(block_number) {
        Ok(Some(block)) => (StatusCode::OK, Json(block)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": format!("Block #{} not found", block_number)
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Database error: {}", e)
            })),
        )
            .into_response(),
    }
}
