//! Shared test helpers — seed users, accounts, agents and mint JWTs the
//! way `sqlx::test`-driven handler tests need them.
//!
//! Only compiled under `cfg(test)`, so this module doesn't ship to
//! production binaries. The `pub(crate)` items are reachable from every
//! handler test module via `crate::tests_support::*`.

use chakramcp_shared::config::SharedConfig;
use chakramcp_shared::jwt;
use sqlx::PgPool;
use uuid::Uuid;

pub(crate) const TEST_SECRET: &str = "test-secret-test-secret-test-secret-test-secret";

pub(crate) fn test_config() -> SharedConfig {
    SharedConfig {
        database_url: "ignored".into(),
        jwt_secret: TEST_SECRET.into(),
        admin_email: None,
        survey_enabled: false,
        frontend_base_url: "http://localhost:3000".into(),
        app_base_url: "http://localhost:8080".into(),
        relay_base_url: "http://localhost:8090".into(),
        discovery_v2_enabled: false,
        log_filter: "warn".into(),
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SeededUser {
    pub user_id: Uuid,
}

/// Seed a user + their personal account + owner membership, and return
/// the user, a freshly-minted JWT, and the personal account id.
///
/// `handle` becomes both the email local-part and the slug, so the
/// account is addressable as `{handle}` in URL paths.
pub(crate) async fn seed_user_with_personal(
    pool: &PgPool,
    handle: &str,
) -> (SeededUser, String, Uuid) {
    let user_id = Uuid::now_v7();
    let email = format!("{handle}-{}@t.local", Uuid::now_v7().simple());
    sqlx::query!(
        r#"
        INSERT INTO users (id, email, display_name, password_hash)
        VALUES ($1, $2, $3, 'x')
        "#,
        user_id,
        email,
        handle,
    )
    .execute(pool)
    .await
    .unwrap();

    let account_id = Uuid::now_v7();
    let slug = format!("{handle}-{}", &Uuid::now_v7().simple().to_string()[..8]);
    sqlx::query!(
        r#"
        INSERT INTO accounts (id, slug, display_name, account_type, owner_user_id)
        VALUES ($1, $2, $3, 'individual', $4)
        "#,
        account_id,
        slug,
        handle,
        user_id,
    )
    .execute(pool)
    .await
    .unwrap();

    sqlx::query!(
        r#"
        INSERT INTO account_memberships (id, account_id, user_id, role)
        VALUES ($1, $2, $3, 'owner')
        "#,
        Uuid::now_v7(),
        account_id,
        user_id,
    )
    .execute(pool)
    .await
    .unwrap();

    let claims = jwt::UserClaims::new(user_id, email, false, 1);
    let token = jwt::encode_jwt(&claims, TEST_SECRET).unwrap();

    (SeededUser { user_id }, token, account_id)
}

/// Seed an organisation account owned by `owner_id` with `owner` membership.
pub(crate) async fn seed_org(pool: &PgPool, slug: &str, owner_id: Uuid) -> Uuid {
    let account_id = Uuid::now_v7();
    sqlx::query!(
        r#"
        INSERT INTO accounts (id, slug, display_name, account_type, owner_user_id)
        VALUES ($1, $2, $3, 'organization', $4)
        "#,
        account_id,
        slug,
        slug,
        owner_id,
    )
    .execute(pool)
    .await
    .unwrap();

    sqlx::query!(
        r#"
        INSERT INTO account_memberships (id, account_id, user_id, role)
        VALUES ($1, $2, $3, 'owner')
        "#,
        Uuid::now_v7(),
        account_id,
        owner_id,
    )
    .execute(pool)
    .await
    .unwrap();

    account_id
}

/// Seed an agent. Returns the agent's UUID.
pub(crate) async fn seed_agent(
    pool: &PgPool,
    account_id: Uuid,
    slug: &str,
    created_by: Uuid,
) -> Uuid {
    let agent_id = Uuid::now_v7();
    sqlx::query!(
        r#"
        INSERT INTO agents
            (id, account_id, created_by_user_id, slug, display_name, description, visibility, mode)
        VALUES ($1, $2, $3, $4, $5, '', 'private', 'pull')
        "#,
        agent_id,
        account_id,
        created_by,
        slug,
        slug,
    )
    .execute(pool)
    .await
    .unwrap();
    agent_id
}

/// Seed an API key for `user_id`. Returns (key_id, plaintext).
pub(crate) async fn seed_api_key(pool: &PgPool, user_id: Uuid, name: &str) -> (Uuid, String) {
    let plaintext = crate::auth::generate_api_key();
    let key_hash = crate::auth::hash_api_key(&plaintext);
    let prefix = crate::auth::key_prefix(&plaintext);
    let id = Uuid::now_v7();
    sqlx::query!(
        r#"
        INSERT INTO api_keys (id, user_id, name, key_hash, key_prefix)
        VALUES ($1, $2, $3, $4, $5)
        "#,
        id,
        user_id,
        name,
        key_hash,
        prefix,
    )
    .execute(pool)
    .await
    .unwrap();
    (id, plaintext)
}

/// Seed an approved-and-not-yet-consumed device-flow row. Returns the
/// row id. Useful for /v1/pairings list + revoke tests.
pub(crate) async fn seed_approved_device_flow(
    pool: &PgPool,
    user_id: Uuid,
    agent_id: Uuid,
) -> Uuid {
    let id = Uuid::now_v7();
    let device_code_hash = format!("hash-{}", Uuid::now_v7().simple());
    let user_code = format!("USRC-{}", &Uuid::now_v7().simple().to_string()[..4]);
    sqlx::query!(
        r#"
        INSERT INTO oauth_device_codes
            (id, device_code_hash, user_code, expires_at,
             approved_user_id, approved_agent_id, approved_at)
        VALUES ($1, $2, $3, now() + interval '10 minutes',
                $4, $5, now())
        "#,
        id,
        device_code_hash,
        user_code,
        user_id,
        agent_id,
    )
    .execute(pool)
    .await
    .unwrap();
    id
}
