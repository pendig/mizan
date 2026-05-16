use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use mizan_core::{AppError, AppResult, DatabaseBackend, ErrorEnvelope};
use mizan_metering::UsageChargeInput;
use mizan_providers::{ChatMessage, TokenUsage};
use mizan_wallet::{RoutePrice, calculate_usage_charge};
use serde::{Deserialize, Serialize};
use sqlx::{Any, AnyPool, FromRow, Transaction, query, query_as};
use uuid::Uuid;

use crate::AppState;
use crate::auth::ApiKeyIdentity;
use crate::utils::{from_app_error, prepare_sql, unix_timestamp_string};

const DEFAULT_USAGE_LIST_LIMIT: i64 = 100;
const MAX_USAGE_LIST_LIMIT: i64 = 500;
const CREDIT_GRANT_REASON: &str = "credit_grant";
const USAGE_CHARGE_REASON: &str = "usage_charge";

type BillingHttpResult<T> = Result<T, (StatusCode, Json<ErrorEnvelope>)>;

#[derive(Debug, Clone, Serialize)]
pub struct WalletResponse {
    pub user_id: Uuid,
    pub balance_microcredits: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct UsageEventResponse {
    pub id: Uuid,
    pub request_id: Uuid,
    pub user_id: Option<Uuid>,
    pub api_key_id: Option<Uuid>,
    pub provider_id: Option<Uuid>,
    pub route_id: Option<Uuid>,
    pub model: String,
    pub usage_prompt_tokens: u64,
    pub usage_completion_tokens: u64,
    pub usage_total_tokens: u64,
    pub usage_estimated: bool,
    pub status_code: u16,
    pub latency_ms: u64,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct UsageEventListResponse {
    pub data: Vec<UsageEventResponse>,
}

#[derive(Debug, Deserialize)]
pub struct UsageQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct AdminUsageQuery {
    pub user_id: Option<Uuid>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct GrantRequest {
    pub amount_microcredits: i64,
    pub reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct GrantResponse {
    pub user_id: Uuid,
    pub wallet_id: Uuid,
    pub balance_microcredits: i64,
    pub amount_microcredits: i64,
    pub reason: String,
}

#[derive(Debug)]
pub struct BillingInput {
    pub request_id: Uuid,
    pub user_id: Uuid,
    pub api_key_id: Option<Uuid>,
    pub provider_id: Option<Uuid>,
    pub route_id: Option<Uuid>,
    pub model: String,
    pub usage: TokenUsage,
    pub status_code: u16,
    pub latency_ms: u64,
    pub route_price: RoutePrice,
}

pub fn estimate_usage(prompt_messages: &[ChatMessage], completion_text: &str) -> TokenUsage {
    let prompt_bytes = prompt_messages
        .iter()
        .map(|message| message.content.len() as u64)
        .sum::<u64>();
    let completion_bytes = completion_text.len() as u64;

    let prompt_tokens = ceil_divide(prompt_bytes, 4);
    let completion_tokens = ceil_divide(completion_bytes, 4);

    TokenUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens: prompt_tokens.saturating_add(completion_tokens),
        estimated: true,
    }
}

pub async fn get_wallet(
    State(state): State<AppState>,
    Extension(identity): Extension<ApiKeyIdentity>,
) -> BillingHttpResult<Json<WalletResponse>> {
    let wallet = ensure_wallet(&state.database, state.database_backend(), identity.user_id)
        .await
        .map_err(from_app_error)?;

    Ok(Json(WalletResponse {
        user_id: wallet.owner_user_id,
        balance_microcredits: wallet.balance_microcredits,
    }))
}

pub async fn list_usage(
    State(state): State<AppState>,
    Extension(identity): Extension<ApiKeyIdentity>,
    Query(query): Query<UsageQuery>,
) -> BillingHttpResult<Json<UsageEventListResponse>> {
    let rows = list_usage_events(
        &state.database,
        state.database_backend(),
        Some(identity.user_id),
        query.limit,
        query.offset,
    )
    .await
    .map_err(from_app_error)?;

    Ok(Json(UsageEventListResponse { data: rows }))
}

pub async fn list_usage_admin(
    State(state): State<AppState>,
    Query(query): Query<AdminUsageQuery>,
) -> BillingHttpResult<Json<UsageEventListResponse>> {
    let rows = list_usage_events(
        &state.database,
        state.database_backend(),
        query.user_id,
        query.limit,
        query.offset,
    )
    .await
    .map_err(from_app_error)?;

    Ok(Json(UsageEventListResponse { data: rows }))
}

pub async fn grant_credits(
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
    Json(payload): Json<GrantRequest>,
) -> BillingHttpResult<Json<GrantResponse>> {
    if payload.amount_microcredits <= 0 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorEnvelope::from(&AppError::invalid_config(
                "grant.amount_microcredits",
                "amount_microcredits must be greater than 0",
            ))),
        ));
    }

    let reason = payload
        .reason
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| CREDIT_GRANT_REASON.to_owned());

    let mut tx = state.database.begin().await.map_err(|error| {
        from_app_error(AppError::infrastructure(format!(
            "failed to begin grant credits transaction: {error}"
        )))
    })?;

    let wallet = ensure_wallet_in_tx(&mut tx, state.database_backend(), user_id)
        .await
        .map_err(from_app_error)?;

    let now = unix_timestamp_string();
    let new_balance = wallet
        .balance_microcredits
        .saturating_add(payload.amount_microcredits);
    let tx_connection = tx.as_mut();

    query(&prepare_sql(
        state.database_backend(),
        "INSERT INTO credit_ledger (
                id,
                wallet_id,
                request_id,
                request_delta_microcredits,
                balance_after_microcredits,
                reason,
                created_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?)",
    ))
    .bind(Uuid::now_v7().to_string())
    .bind(wallet.id.to_string())
    .bind(Option::<String>::None)
    .bind(payload.amount_microcredits)
    .bind(new_balance)
    .bind(&reason)
    .bind(&now)
    .execute(&mut *tx_connection)
    .await
    .map_err(|error| {
        AppError::infrastructure(format!("failed to create credit ledger entry: {error}"))
    })
    .map_err(from_app_error)?;

    query(&prepare_sql(
        state.database_backend(),
        "UPDATE wallets SET balance_microcredits = ?, updated_at = ? WHERE id = ?",
    ))
    .bind(new_balance)
    .bind(&now)
    .bind(wallet.id.to_string())
    .execute(&mut *tx_connection)
    .await
    .map_err(|error| AppError::infrastructure(format!("failed to update wallet balance: {error}")))
    .map_err(from_app_error)?;

    tx.commit().await.map_err(|error| {
        from_app_error(AppError::infrastructure(format!(
            "failed to commit credit grant transaction: {error}"
        )))
    })?;

    Ok(Json(GrantResponse {
        user_id,
        wallet_id: wallet.id,
        amount_microcredits: payload.amount_microcredits,
        balance_microcredits: new_balance,
        reason,
    }))
}

pub async fn record_usage(
    database: &AnyPool,
    database_backend: DatabaseBackend,
    input: BillingInput,
) -> AppResult<()> {
    if !(200..=299).contains(&input.status_code) {
        insert_usage_event(database, database_backend, &input, input.status_code).await?;
        return Ok(());
    }

    let charge = calculate_usage_charge(
        UsageChargeInput {
            prompt_tokens: input.usage.prompt_tokens,
            completion_tokens: input.usage.completion_tokens,
        },
        input.route_price,
    );

    if charge.total.0 <= 0 {
        insert_usage_event(database, database_backend, &input, input.status_code).await?;
        return Ok(());
    }

    let mut tx = database.begin().await.map_err(|error| {
        AppError::infrastructure(format!("cannot open transaction for metering: {error}"))
    })?;

    let wallet = ensure_wallet_in_tx(&mut tx, database_backend, input.user_id).await?;

    if wallet.balance_microcredits < charge.total.0 {
        insert_usage_event(
            database,
            database_backend,
            &BillingInput {
                status_code: StatusCode::PAYMENT_REQUIRED.as_u16(),
                ..input
            },
            StatusCode::PAYMENT_REQUIRED.as_u16(),
        )
        .await?;

        return Err(AppError::InsufficientCredit);
    }

    let new_balance = wallet.balance_microcredits - charge.total.0;
    let now = unix_timestamp_string();
    let tx_connection = tx.as_mut();

    query(&prepare_sql(
        database_backend,
        "INSERT INTO credit_ledger (
                id,
                wallet_id,
                request_id,
                request_delta_microcredits,
                balance_after_microcredits,
                reason,
                created_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?)",
    ))
    .bind(Uuid::now_v7().to_string())
    .bind(wallet.id.to_string())
    .bind(input.request_id.to_string())
    .bind(-charge.total.0)
    .bind(new_balance)
    .bind(USAGE_CHARGE_REASON)
    .bind(&now)
    .execute(&mut *tx_connection)
    .await
    .map_err(|error| {
        AppError::infrastructure(format!("failed to create credit ledger entry: {error}"))
    })?;

    query(&prepare_sql(
        database_backend,
        "UPDATE wallets SET balance_microcredits = ?, updated_at = ? WHERE id = ?",
    ))
    .bind(new_balance)
    .bind(&now)
    .bind(wallet.id.to_string())
    .execute(tx_connection)
    .await
    .map_err(|error| {
        AppError::infrastructure(format!("failed to update wallet balance: {error}"))
    })?;

    insert_usage_event_tx(&mut tx, database_backend, &input, input.status_code).await?;

    tx.commit().await.map_err(|error| {
        AppError::infrastructure(format!("failed to commit metering transaction: {error}"))
    })?;

    Ok(())
}

pub async fn ensure_sufficient_credit(
    database: &AnyPool,
    database_backend: DatabaseBackend,
    user_id: Uuid,
    usage: TokenUsage,
    route_price: RoutePrice,
) -> AppResult<()> {
    let charge = calculate_usage_charge(
        UsageChargeInput {
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
        },
        route_price,
    );

    if charge.total.0 <= 0 {
        return Ok(());
    }

    let wallet = ensure_wallet(database, database_backend, user_id).await?;
    if wallet.balance_microcredits < charge.total.0 {
        return Err(AppError::InsufficientCredit);
    }

    Ok(())
}

#[derive(Debug, Clone)]
struct WalletRow {
    id: Uuid,
    owner_user_id: Uuid,
    balance_microcredits: i64,
}

fn normalize_limit(limit: Option<i64>) -> AppResult<i64> {
    let value = limit.unwrap_or(DEFAULT_USAGE_LIST_LIMIT);
    if value <= 0 {
        return Err(AppError::invalid_config(
            "usage.limit",
            "limit must be positive",
        ));
    }

    Ok(value.min(MAX_USAGE_LIST_LIMIT))
}

fn normalize_offset(offset: Option<i64>) -> i64 {
    offset.unwrap_or_default().max(0)
}

fn parse_status_code(value: i64) -> AppResult<u16> {
    u16::try_from(value).map_err(|error| {
        AppError::infrastructure(format!("invalid status code in usage event: {error}"))
    })
}

fn parse_optional_uuid(value: Option<&String>) -> AppResult<Option<Uuid>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }

    Ok(Some(Uuid::parse_str(value).map_err(|error| {
        AppError::infrastructure(format!("invalid uuid in usage event payload: {error}"))
    })?))
}

fn to_status_code(value: i64) -> AppResult<u16> {
    parse_status_code(value)
}

fn to_u64(value: i64) -> AppResult<u64> {
    u64::try_from(value).map_err(|error| {
        AppError::infrastructure(format!(
            "invalid numeric value in usage event payload: {error}"
        ))
    })
}

async fn list_usage_events(
    database: &AnyPool,
    database_backend: DatabaseBackend,
    user_id_filter: Option<Uuid>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> AppResult<Vec<UsageEventResponse>> {
    let limit = normalize_limit(limit)?;
    let offset = normalize_offset(offset);

    #[derive(Debug, FromRow)]
    struct UsageEventRow {
        id: String,
        request_id: String,
        user_id: Option<String>,
        api_key_id: Option<String>,
        provider_id: Option<String>,
        route_id: Option<String>,
        model: String,
        usage_prompt_tokens: i64,
        usage_completion_tokens: i64,
        usage_total_tokens: i64,
        usage_estimated: i64,
        status_code: i64,
        latency_ms: i64,
        created_at: String,
    }

    let sql = if user_id_filter.is_some() {
        "SELECT id, request_id, user_id, api_key_id, provider_id, route_id, model,\
             usage_prompt_tokens, usage_completion_tokens, usage_total_tokens, usage_estimated,\
             status_code, latency_ms, created_at\
             FROM usage_events\
             WHERE user_id = ?\
             ORDER BY created_at DESC\
             LIMIT ? OFFSET ?"
    } else {
        "SELECT id, request_id, user_id, api_key_id, provider_id, route_id, model,\
             usage_prompt_tokens, usage_completion_tokens, usage_total_tokens, usage_estimated,\
             status_code, latency_ms, created_at\
             FROM usage_events\
             ORDER BY created_at DESC\
             LIMIT ? OFFSET ?"
    };

    let prepared_sql = prepare_sql(database_backend, sql);
    let mut query = query_as::<_, UsageEventRow>(&prepared_sql);
    if let Some(user_id) = user_id_filter {
        query = query.bind(user_id.to_string());
    }

    let rows = query
        .bind(limit)
        .bind(offset)
        .fetch_all(database)
        .await
        .map_err(|error| AppError::infrastructure(error.to_string()))?;

    rows.into_iter()
        .map(|row| -> AppResult<UsageEventResponse> {
            Ok(UsageEventResponse {
                id: Uuid::parse_str(&row.id).map_err(|error| {
                    AppError::infrastructure(format!("invalid usage event id: {error}"))
                })?,
                request_id: Uuid::parse_str(&row.request_id).map_err(|error| {
                    AppError::infrastructure(format!("invalid request id in usage event: {error}"))
                })?,
                user_id: parse_optional_uuid(row.user_id.as_ref())?,
                api_key_id: parse_optional_uuid(row.api_key_id.as_ref())?,
                provider_id: parse_optional_uuid(row.provider_id.as_ref())?,
                route_id: parse_optional_uuid(row.route_id.as_ref())?,
                model: row.model,
                usage_prompt_tokens: to_u64(row.usage_prompt_tokens)?,
                usage_completion_tokens: to_u64(row.usage_completion_tokens)?,
                usage_total_tokens: to_u64(row.usage_total_tokens)?,
                usage_estimated: row.usage_estimated != 0,
                status_code: to_status_code(row.status_code)?,
                latency_ms: to_u64(row.latency_ms)?,
                created_at: row.created_at,
            })
        })
        .collect::<AppResult<Vec<_>>>()
}

async fn ensure_wallet(
    database: &AnyPool,
    database_backend: DatabaseBackend,
    user_id: Uuid,
) -> AppResult<WalletRow> {
    let mut tx = database
        .begin()
        .await
        .map_err(|error| AppError::infrastructure(error.to_string()))?;

    let wallet = ensure_wallet_in_tx(&mut tx, database_backend, user_id).await?;

    tx.commit().await.map_err(|error| {
        AppError::infrastructure(format!("failed to persist wallet row: {error}"))
    })?;

    Ok(wallet)
}

async fn ensure_wallet_in_tx(
    tx: &mut Transaction<'_, Any>,
    database_backend: DatabaseBackend,
    user_id: Uuid,
) -> AppResult<WalletRow> {
    let lock_sql = match database_backend {
        DatabaseBackend::Sqlite => {
            "SELECT id, balance_microcredits FROM wallets WHERE owner_user_id = ? ORDER BY created_at DESC LIMIT 1"
        }
        DatabaseBackend::Postgres => {
            "SELECT id, balance_microcredits FROM wallets WHERE owner_user_id = ? ORDER BY created_at DESC LIMIT 1 FOR UPDATE"
        }
    };

    let connection = tx.as_mut();

    let existing = query_as::<_, (String, i64)>(&prepare_sql(database_backend, lock_sql))
        .bind(user_id.to_string())
        .fetch_optional(&mut *connection)
        .await
        .map_err(|error| AppError::infrastructure(error.to_string()))?;

    if let Some((id, balance_microcredits)) = existing {
        let id = Uuid::parse_str(&id)
            .map_err(|error| AppError::infrastructure(format!("invalid wallet id: {error}")))?;

        return Ok(WalletRow {
            id,
            owner_user_id: user_id,
            balance_microcredits,
        });
    }

    let wallet_id = Uuid::now_v7();
    let now = unix_timestamp_string();
    let insert_sql = match database_backend {
        DatabaseBackend::Sqlite => {
            "INSERT INTO wallets (id, owner_user_id, balance_microcredits, created_at, updated_at) \
             VALUES (?, ?, 0, ?, ?) ON CONFLICT(owner_user_id) DO NOTHING"
        }
        DatabaseBackend::Postgres => {
            "INSERT INTO wallets (id, owner_user_id, balance_microcredits, created_at, updated_at) \
             VALUES (?, ?, 0, ?, ?) ON CONFLICT(owner_user_id) DO NOTHING"
        }
    };

    query(&prepare_sql(database_backend, insert_sql))
        .bind(wallet_id.to_string())
        .bind(user_id.to_string())
        .bind(&now)
        .bind(&now)
        .execute(&mut *connection)
        .await
        .map_err(|error| AppError::infrastructure(format!("cannot initialize wallet: {error}")))?;

    let existing = query_as::<_, (String, i64)>(&prepare_sql(database_backend, lock_sql))
        .bind(user_id.to_string())
        .fetch_optional(&mut *connection)
        .await
        .map_err(|error| AppError::infrastructure(error.to_string()))?
        .ok_or_else(|| {
            AppError::infrastructure("wallet initialization did not result in a wallet row")
        })?;
    let (id, balance_microcredits) = existing;
    let id = Uuid::parse_str(&id)
        .map_err(|error| AppError::infrastructure(format!("invalid wallet id: {error}")))?;

    Ok(WalletRow {
        id,
        owner_user_id: user_id,
        balance_microcredits,
    })
}

async fn insert_usage_event(
    database: &AnyPool,
    database_backend: DatabaseBackend,
    input: &BillingInput,
    status_code: u16,
) -> AppResult<()> {
    let now = unix_timestamp_string();

    query(&prepare_sql(
        database_backend,
        "INSERT INTO usage_events (
            id,
            request_id,
            user_id,
            api_key_id,
            provider_id,
            route_id,
            model,
            usage_prompt_tokens,
            usage_completion_tokens,
            usage_total_tokens,
            usage_estimated,
            status_code,
            latency_ms,
            created_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    ))
    .bind(Uuid::now_v7().to_string())
    .bind(input.request_id.to_string())
    .bind(input.user_id.to_string())
    .bind(input.api_key_id.map(|value| value.to_string()))
    .bind(input.provider_id.map(|value| value.to_string()))
    .bind(input.route_id.map(|value| value.to_string()))
    .bind(&input.model)
    .bind(i64::try_from(input.usage.prompt_tokens).map_err(|_| {
        AppError::invalid_config("usage.prompt_tokens", "prompt tokens must fit into i64")
    })?)
    .bind(i64::try_from(input.usage.completion_tokens).map_err(|_| {
        AppError::invalid_config(
            "usage.completion_tokens",
            "completion tokens must fit into i64",
        )
    })?)
    .bind(i64::try_from(input.usage.total_tokens).map_err(|_| {
        AppError::invalid_config("usage.total_tokens", "total tokens must fit into i64")
    })?)
    .bind(i64::from(input.usage.estimated))
    .bind(i64::from(status_code))
    .bind(
        i64::try_from(input.latency_ms).map_err(|_| {
            AppError::invalid_config("usage.latency_ms", "latency must fit into i64")
        })?,
    )
    .bind(&now)
    .execute(database)
    .await
    .map_err(|error| {
        AppError::infrastructure(format!(
            "failed to insert usage event for request_id={} : {error}",
            input.request_id
        ))
    })?;

    Ok(())
}

async fn insert_usage_event_tx(
    tx: &mut Transaction<'_, Any>,
    database_backend: DatabaseBackend,
    input: &BillingInput,
    status_code: u16,
) -> AppResult<()> {
    let now = unix_timestamp_string();
    let connection = tx.as_mut();

    query(&prepare_sql(
        database_backend,
        "INSERT INTO usage_events (
            id,
            request_id,
            user_id,
            api_key_id,
            provider_id,
            route_id,
            model,
            usage_prompt_tokens,
            usage_completion_tokens,
            usage_total_tokens,
            usage_estimated,
            status_code,
            latency_ms,
            created_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    ))
    .bind(Uuid::now_v7().to_string())
    .bind(input.request_id.to_string())
    .bind(input.user_id.to_string())
    .bind(input.api_key_id.map(|value| value.to_string()))
    .bind(input.provider_id.map(|value| value.to_string()))
    .bind(input.route_id.map(|value| value.to_string()))
    .bind(&input.model)
    .bind(i64::try_from(input.usage.prompt_tokens).map_err(|_| {
        AppError::invalid_config("usage.prompt_tokens", "prompt tokens must fit into i64")
    })?)
    .bind(i64::try_from(input.usage.completion_tokens).map_err(|_| {
        AppError::invalid_config(
            "usage.completion_tokens",
            "completion tokens must fit into i64",
        )
    })?)
    .bind(i64::try_from(input.usage.total_tokens).map_err(|_| {
        AppError::invalid_config("usage.total_tokens", "total tokens must fit into i64")
    })?)
    .bind(i64::from(input.usage.estimated))
    .bind(i64::from(status_code))
    .bind(
        i64::try_from(input.latency_ms).map_err(|_| {
            AppError::invalid_config("usage.latency_ms", "latency must fit into i64")
        })?,
    )
    .bind(&now)
    .execute(connection)
    .await
    .map_err(|error| {
        AppError::infrastructure(format!(
            "failed to insert usage event for request_id={} : {error}",
            input.request_id
        ))
    })?;

    Ok(())
}

fn ceil_divide(dividend: u64, divisor: u64) -> u64 {
    if dividend == 0 {
        return 0;
    }

    dividend.div_ceil(divisor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_usage_is_tokenized_roughly_by_4_bytes() {
        let prompt = vec![ChatMessage {
            role: "user".to_owned(),
            content: "hello world".to_owned(),
        }];

        let usage = estimate_usage(&prompt, "hi there");
        assert!(usage.estimated);
        assert_eq!(usage.prompt_tokens, 3);
        assert_eq!(usage.completion_tokens, 2);
        assert_eq!(usage.total_tokens, 5);
    }

    #[test]
    fn normalize_usage_limits_are_enforced() {
        assert!(normalize_limit(Some(0)).is_err());
        assert_eq!(normalize_limit(Some(1000)).unwrap(), MAX_USAGE_LIST_LIMIT);
    }
}
