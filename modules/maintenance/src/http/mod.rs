pub mod assets;
pub mod calibration_events;
pub mod downtime;
pub mod health;
pub mod meters;
pub mod plans;
pub mod work_order_labor;
pub mod work_order_parts;
pub mod work_orders;

use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub error: String,
    pub message: String,
}

impl ErrorBody {
    pub fn new(error: &str, message: &str) -> Self {
        Self {
            error: error.to_string(),
            message: message.to_string(),
        }
    }
}
