use mizan_core::{AppError, AppResult};
use redis::Client;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LimitScope {
    ApiKey(Uuid),
    User(Uuid),
    Provider(Uuid),
}

impl LimitScope {
    fn key_part(self) -> String {
        match self {
            Self::ApiKey(id) => format!("api_key:{id}"),
            Self::User(id) => format!("user:{id}"),
            Self::Provider(id) => format!("provider:{id}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LimitDecision {
    Allowed,
    Blocked { reason: String },
}

impl LimitDecision {
    pub fn allowed(&self) -> bool {
        matches!(self, Self::Allowed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeLimitPolicy {
    pub requests_per_window: u32,
    pub tokens_per_window: u32,
    pub concurrent_requests: u32,
    pub window_seconds: u32,
    pub lease_seconds: u32,
}

impl RuntimeLimitPolicy {
    pub fn disabled() -> Self {
        Self {
            requests_per_window: 0,
            tokens_per_window: 0,
            concurrent_requests: 0,
            window_seconds: 60,
            lease_seconds: 120,
        }
    }

    pub fn is_disabled(&self) -> bool {
        self.requests_per_window == 0
            && self.tokens_per_window == 0
            && self.concurrent_requests == 0
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeLimitRequest {
    pub scopes: Vec<LimitScope>,
    pub estimated_prompt_tokens: u64,
}

#[derive(Debug)]
pub struct RuntimeLimitLease {
    client: Client,
    keys: Vec<String>,
}

impl RuntimeLimitLease {
    pub fn noop(client: Client) -> Self {
        Self {
            client,
            keys: Vec::new(),
        }
    }

    pub fn release(self) -> AppResult<()> {
        if self.keys.is_empty() {
            return Ok(());
        }

        let mut connection = self
            .client
            .get_connection()
            .map_err(|error| AppError::infrastructure(error.to_string()))?;
        release_keys(&mut connection, &self.keys)
    }
}

pub fn check_and_acquire(
    client: Client,
    policy: RuntimeLimitPolicy,
    request: RuntimeLimitRequest,
) -> AppResult<RuntimeLimitLease> {
    if policy.is_disabled() || request.scopes.is_empty() {
        return Ok(RuntimeLimitLease::noop(client));
    }

    let mut connection = client
        .get_connection()
        .map_err(|error| AppError::infrastructure(error.to_string()))?;
    let mut acquired_leases = Vec::new();

    for scope in request.scopes {
        if let Err(error) = check_request_counter(&mut connection, policy, scope) {
            let _ = release_keys(&mut connection, &acquired_leases);
            return Err(error);
        }
        if let Err(error) = check_token_counter(
            &mut connection,
            policy,
            scope,
            request.estimated_prompt_tokens,
        ) {
            let _ = release_keys(&mut connection, &acquired_leases);
            return Err(error);
        }
        match acquire_concurrency_lease(&mut connection, policy, scope) {
            Ok(Some(key)) => acquired_leases.push(key),
            Ok(None) => {}
            Err(error) => {
                let _ = release_keys(&mut connection, &acquired_leases);
                return Err(error);
            }
        }
    }

    Ok(RuntimeLimitLease {
        client,
        keys: acquired_leases,
    })
}

fn check_request_counter(
    connection: &mut redis::Connection,
    policy: RuntimeLimitPolicy,
    scope: LimitScope,
) -> AppResult<()> {
    if policy.requests_per_window == 0 {
        return Ok(());
    }

    let key = format!("mizan:limit:rpm:{}", scope.key_part());
    let count = increment_window_counter(connection, &key, policy.window_seconds, 1)?;
    if count > u64::from(policy.requests_per_window) {
        return Err(AppError::LimitExceeded(format!(
            "requests_per_window exceeded for {}",
            scope.key_part()
        )));
    }

    Ok(())
}

fn check_token_counter(
    connection: &mut redis::Connection,
    policy: RuntimeLimitPolicy,
    scope: LimitScope,
    tokens: u64,
) -> AppResult<()> {
    if policy.tokens_per_window == 0 || tokens == 0 {
        return Ok(());
    }

    let key = format!("mizan:limit:tpm:{}", scope.key_part());
    let count = increment_window_counter(connection, &key, policy.window_seconds, tokens)?;
    if count > u64::from(policy.tokens_per_window) {
        return Err(AppError::LimitExceeded(format!(
            "tokens_per_window exceeded for {}",
            scope.key_part()
        )));
    }

    Ok(())
}

fn acquire_concurrency_lease(
    connection: &mut redis::Connection,
    policy: RuntimeLimitPolicy,
    scope: LimitScope,
) -> AppResult<Option<String>> {
    if policy.concurrent_requests == 0 {
        return Ok(None);
    }

    let counter_key = format!("mizan:limit:concurrency:{}", scope.key_part());
    let count = increment_window_counter(connection, &counter_key, policy.lease_seconds, 1)?;
    if count > u64::from(policy.concurrent_requests) {
        release_key(connection, &counter_key)?;
        return Err(AppError::LimitExceeded(format!(
            "concurrent_requests exceeded for {}",
            scope.key_part()
        )));
    }

    Ok(Some(counter_key))
}

fn increment_window_counter(
    connection: &mut redis::Connection,
    key: &str,
    ttl_seconds: u32,
    amount: u64,
) -> AppResult<u64> {
    let count: u64 = redis::cmd("EVAL")
        .arg(
            "local current = redis.call('INCRBY', KEYS[1], ARGV[1]) \
             if current == tonumber(ARGV[1]) then \
               redis.call('EXPIRE', KEYS[1], ARGV[2]) \
             end \
             return current",
        )
        .arg(1)
        .arg(key)
        .arg(amount)
        .arg(ttl_seconds.max(1))
        .query(connection)
        .map_err(|error| AppError::infrastructure(error.to_string()))?;

    Ok(count)
}

fn release_key(connection: &mut redis::Connection, key: &str) -> AppResult<i64> {
    let count: i64 = redis::cmd("EVAL")
        .arg(
            "if redis.call('EXISTS', KEYS[1]) == 0 then \
               return 0 \
             end \
             local current = redis.call('DECR', KEYS[1]) \
             if current <= 0 then \
               redis.call('DEL', KEYS[1]) \
             end \
             return current",
        )
        .arg(1)
        .arg(key)
        .query(connection)
        .map_err(|error| AppError::infrastructure(error.to_string()))?;

    Ok(count)
}

fn release_keys(connection: &mut redis::Connection, keys: &[String]) -> AppResult<()> {
    let mut first_error = None;

    for key in keys {
        if let Err(error) = release_key(connection, key)
            && first_error.is_none()
        {
            first_error = Some(error);
        }
    }

    if let Some(error) = first_error {
        return Err(error);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use redis::Commands;

    #[test]
    fn disabled_policy_is_detected() {
        assert!(RuntimeLimitPolicy::disabled().is_disabled());
    }

    #[test]
    fn scope_keys_are_stable() {
        let id = Uuid::parse_str("018f0d6f-4fd4-7b0d-9c3d-fec9dd31e906").expect("uuid");

        assert_eq!(
            LimitScope::ApiKey(id).key_part(),
            "api_key:018f0d6f-4fd4-7b0d-9c3d-fec9dd31e906"
        );
        assert_eq!(
            LimitScope::User(id).key_part(),
            "user:018f0d6f-4fd4-7b0d-9c3d-fec9dd31e906"
        );
        assert_eq!(
            LimitScope::Provider(id).key_part(),
            "provider:018f0d6f-4fd4-7b0d-9c3d-fec9dd31e906"
        );
    }

    fn redis_client_from_env() -> redis::RedisResult<Client> {
        let redis_url =
            std::env::var("MIZAN_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
        Client::open(redis_url)
    }

    fn cleanup_keys(client: &Client, keys: &[String]) {
        if let Ok(mut connection) = client.get_connection() {
            for key in keys {
                let _ = redis::cmd("DEL").arg(key).query::<i64>(&mut connection);
            }
        }
    }

    fn test_policy() -> RuntimeLimitPolicy {
        RuntimeLimitPolicy {
            requests_per_window: 2,
            tokens_per_window: 5,
            concurrent_requests: 1,
            window_seconds: 60,
            lease_seconds: 30,
        }
    }

    #[test]
    #[ignore = "requires a reachable Redis instance; run with scripts/limit-smoke.sh"]
    fn redis_rpm_blocks_after_window_limit() {
        let client = redis_client_from_env().expect("redis client");
        let scope = LimitScope::ApiKey(Uuid::now_v7());
        let key = format!("mizan:limit:rpm:{}", scope.key_part());
        cleanup_keys(&client, std::slice::from_ref(&key));

        let policy = RuntimeLimitPolicy {
            requests_per_window: 2,
            tokens_per_window: 0,
            concurrent_requests: 0,
            window_seconds: 60,
            lease_seconds: 30,
        };

        for _ in 0..2 {
            check_and_acquire(
                client.clone(),
                policy,
                RuntimeLimitRequest {
                    scopes: vec![scope],
                    estimated_prompt_tokens: 0,
                },
            )
            .expect("request should be allowed");
        }

        let error = check_and_acquire(
            client.clone(),
            policy,
            RuntimeLimitRequest {
                scopes: vec![scope],
                estimated_prompt_tokens: 0,
            },
        )
        .expect_err("third request should be rate limited");

        assert!(matches!(error, AppError::LimitExceeded(_)));
        cleanup_keys(&client, &[key]);
    }

    #[test]
    #[ignore = "requires a reachable Redis instance; run with scripts/limit-smoke.sh"]
    fn redis_concurrency_release_allows_next_acquire() {
        let client = redis_client_from_env().expect("redis client");
        let scope = LimitScope::User(Uuid::now_v7());
        let key = format!("mizan:limit:concurrency:{}", scope.key_part());
        cleanup_keys(&client, std::slice::from_ref(&key));

        let lease = check_and_acquire(
            client.clone(),
            test_policy(),
            RuntimeLimitRequest {
                scopes: vec![scope],
                estimated_prompt_tokens: 1,
            },
        )
        .expect("first request should acquire concurrency lease");

        let error = check_and_acquire(
            client.clone(),
            test_policy(),
            RuntimeLimitRequest {
                scopes: vec![scope],
                estimated_prompt_tokens: 1,
            },
        )
        .expect_err("second concurrent request should be blocked");
        assert!(matches!(error, AppError::LimitExceeded(_)));

        lease.release().expect("lease release should succeed");

        let next_lease = check_and_acquire(
            client.clone(),
            test_policy(),
            RuntimeLimitRequest {
                scopes: vec![scope],
                estimated_prompt_tokens: 1,
            },
        )
        .expect("released concurrency slot should allow the next request");
        next_lease.release().expect("cleanup next lease");
        cleanup_keys(&client, &[key]);
    }

    #[test]
    #[ignore = "requires a reachable Redis instance; run with scripts/limit-smoke.sh"]
    fn redis_later_scope_failure_releases_earlier_concurrency_lease() {
        let client = redis_client_from_env().expect("redis client");
        let first_scope = LimitScope::ApiKey(Uuid::now_v7());
        let second_scope = LimitScope::Provider(Uuid::now_v7());
        let first_concurrency_key = format!("mizan:limit:concurrency:{}", first_scope.key_part());
        let second_tpm_key = format!("mizan:limit:tpm:{}", second_scope.key_part());
        cleanup_keys(
            &client,
            &[first_concurrency_key.clone(), second_tpm_key.clone()],
        );

        let mut connection = client.get_connection().expect("redis connection");
        redis::cmd("SET")
            .arg(&second_tpm_key)
            .arg(5)
            .arg("EX")
            .arg(60)
            .query::<()>(&mut connection)
            .expect("seed token counter");

        let error = check_and_acquire(
            client.clone(),
            test_policy(),
            RuntimeLimitRequest {
                scopes: vec![first_scope, second_scope],
                estimated_prompt_tokens: 1,
            },
        )
        .expect_err("later scope should exceed token window");
        assert!(matches!(error, AppError::LimitExceeded(_)));

        let remaining: Option<i64> = connection
            .get(&first_concurrency_key)
            .expect("get lease key");
        assert_eq!(
            remaining, None,
            "earlier concurrency lease should be released after later scope failure"
        );
        cleanup_keys(&client, &[first_concurrency_key, second_tpm_key]);
    }

    #[test]
    #[ignore = "requires a reachable Redis instance; run with scripts/limit-smoke.sh"]
    fn redis_release_missing_lease_does_not_create_negative_counter() {
        let client = redis_client_from_env().expect("redis client");
        let key = format!("mizan:limit:concurrency:test-missing:{}", Uuid::now_v7());
        cleanup_keys(&client, std::slice::from_ref(&key));

        RuntimeLimitLease {
            client: client.clone(),
            keys: vec![key.clone()],
        }
        .release()
        .expect("missing lease release should be a no-op");

        let mut connection = client.get_connection().expect("redis connection");
        let remaining: Option<i64> = connection.get(&key).expect("get missing lease key");
        assert_eq!(remaining, None);
        cleanup_keys(&client, &[key]);
    }
}
