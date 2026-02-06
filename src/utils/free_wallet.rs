use sea_orm::{ActiveModelTrait,  ActiveValue::{ Set}};

use crate::{entities::user_wallet, error::error::AppError};

pub async fn mark_wallet_free(
    txn: &sea_orm::DatabaseTransaction,
    wallet: user_wallet::Model,
) -> Result<(), AppError> {
    let mut active: user_wallet::ActiveModel = wallet.into();
    active.status = Set("FREE".to_string());
    active.update(txn).await.map_err(AppError::DbError)?;
    Ok(())
}