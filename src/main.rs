
use crate::{ db::connection::init_db, error::error::AppError,  jobs::index::{ run_sweeper}};
pub mod db;
pub mod error;
pub mod config;
pub mod state_models;
pub mod chain_config;
pub mod jobs;
pub mod entities;
pub mod tokens;
pub mod utils;


#[actix_web::main] 
async fn main() -> Result<(), AppError> {
    // Initialize logging first
    
    let db = init_db().await
        .map_err(|e| {
            tracing::error!("Database initialization failed: {}", e);
            AppError::InternalError(format!("DB init error: {}", e))
        })?;
    
    
    // Use join_all to wait for all workers
    let workers = vec![
        tokio::spawn(run_sweeper(0, db.clone())),
        tokio::spawn(run_sweeper(1, db.clone())),
        tokio::spawn(run_sweeper(2, db.clone())),
        tokio::spawn(run_sweeper(3, db.clone())),
    ];
    
    // Wait for all workers (they should run forever unless error)
    for worker in workers {
        if let Err(e) = worker.await {
            tracing::error!("Worker failed: {:?}", e);
        }
    }
    
    tracing::info!("All workers stopped");
    Ok(())
}

