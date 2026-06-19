use std::sync::Arc;
use tokio::sync::broadcast;

use crate::services::{
    DbService, DynamicService, GridService, PandaScoreService, RampService, SuiService,
    TransakService, UploadService, WalrusService,
};

pub struct AppState {
    pub db: Arc<DbService>,
    pub sui: Arc<SuiService>,
    pub ramp: Arc<RampService>,
    pub transak: Arc<TransakService>,
    pub grid: Arc<GridService>,
    pub pandascore: Arc<PandaScoreService>,
    pub walrus: Arc<WalrusService>,
    pub notif_tx: Arc<broadcast::Sender<(String, serde_json::Value)>>,
    pub dynamic_service: Option<Arc<DynamicService>>,
    pub upload_service: Option<Arc<UploadService>>,
}
