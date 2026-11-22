use std::time::{Duration, Instant};

use corral_core::Error;
use corral_core::domain::{Handle, Role};
use corral_core::ports::{
    Answer, Challenge, Identity, IdentityProvider, ManagedAccounts, Method, PendingLogin,
};
use corral_identity_local::LocalProvider;
use corral_sql::Pool;
use corral_sql::testing::Databases;
use secrecy::SecretString;

const PASSWORD: &str = "correct horse battery";

async fn with(pool: Pool, accounts: &[(&str, Role)]) -> LocalProvider {
    let provider = LocalProvider::open(pool).await.unwrap();
    for &(username, role) in accounts {
        provider
            .create(&handle(username), password(PASSWORD), role)
            .await
            .unwrap();
    }
    provider
}

fn handle(value: &str) -> Handle {
    Handle::parse(value).unwrap()
}

fn password(value: &str) -> SecretString {
    SecretString::from(value)
}

async fn verify(
    provider: &LocalProvider,
    username: &str,
    attempt: &str,
) -> Result<Identity, Error> {
    provider
        .verify(Answer::Password {
            username: username.to_owned(),
            password: password(attempt),
        })
        .await
}

async fn a_password_provider_named_local(pool: Pool) {
    let provider = with(pool, &[]).await;
    assert_eq!(provider.id(), "local");
    assert_eq!(provider.method(), Method::Password);
    assert_eq!(provider.challenge().await.unwrap(), Challenge::Password);
    let redirect = Answer::Redirect {
        pending: PendingLogin("x".to_owned()),
        params: [("code".to_owned(), "x".to_owned())].into(),
    };
    assert!(matches!(
        provider.verify(redirect).await,
        Err(Error::Invalid(_))
    ));
}

async fn the_account_decides_the_role(pool: Pool) {
    let provider = with(pool, &[("adam", Role::Member), ("root", Role::Admin)]).await;
    assert_eq!(
        verify(&provider, "adam", PASSWORD).await.unwrap(),
        Identity {
            subject: "adam".to_owned(),
            username: "adam".to_owned(),
            email: None,
            role: Some(Role::Member),
        }
    );
    assert_eq!(
        verify(&provider, "root", PASSWORD).await.unwrap().role,
        Some(Role::Admin)
    );
}

async fn wrong_password_and_unknown_user_are_both_forbidden(pool: Pool) {
    let provider = with(pool, &[("adam", Role::Member)]).await;
    for (username, attempt) in [
        ("adam", "wrong horse battery"),
        ("nobody", PASSWORD),
        ("Not A Handle", PASSWORD),
        ("Adam", PASSWORD),
    ] {
        let result = verify(&provider, username, attempt).await;
        assert!(
            matches!(result, Err(Error::Forbidden)),
            "{username}: {result:?}"
        );
    }
}

async fn unknown_usernames_cost_a_full_password_check(pool: Pool) {
    let provider = with(pool, &[("adam", Role::Member)]).await;
    let fastest = |username: &'static str| {
        let provider = &provider;
        async move {
            let mut best = Duration::MAX;
            for _ in 0..3 {
                let started = Instant::now();
                let _ = verify(provider, username, "wrong horse battery").await;
                best = best.min(started.elapsed());
            }
            best
        }
    };
    let known = fastest("adam").await;
    for unknown in ["nobody", "Not A Handle"] {
        let elapsed = fastest(unknown).await;
        assert!(elapsed * 4 > known, "{unknown}: {elapsed:?} vs {known:?}");
    }
}

async fn an_existing_username_conflicts(pool: Pool) {
    let provider = with(pool, &[("adam", Role::Member)]).await;
    assert!(matches!(
        provider
            .create(&handle("adam"), password("another password"), Role::Admin)
            .await,
        Err(Error::Conflict(_))
    ));
    assert_eq!(
        verify(&provider, "adam", PASSWORD).await.unwrap().role,
        Some(Role::Member)
    );
}

async fn short_passwords_are_refused(pool: Pool) {
    let provider = with(pool, &[("adam", Role::Member)]).await;
    let short = password("elevenchars");
    assert!(matches!(
        provider
            .create(&handle("eve"), short.clone(), Role::Member)
            .await,
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        provider.set_password("adam", short).await,
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        verify(&provider, "eve", "elevenchars").await,
        Err(Error::Forbidden)
    ));
    assert!(verify(&provider, "adam", PASSWORD).await.is_ok());
}

async fn accounts_are_listed_created_and_reset(pool: Pool) {
    // Byte order puts '-' before digits, unlike most locale collations.
    let provider = with(pool, &[("adam1", Role::Member), ("adam-b", Role::Admin)]).await;
    provider
        .create(&handle("eve"), password(PASSWORD), Role::Admin)
        .await
        .unwrap();
    provider
        .set_password("adam1", password("a brand new password"))
        .await
        .unwrap();
    assert!(matches!(
        verify(&provider, "adam1", PASSWORD).await,
        Err(Error::Forbidden)
    ));
    assert!(
        verify(&provider, "adam1", "a brand new password")
            .await
            .is_ok()
    );
    assert!(verify(&provider, "adam-b", PASSWORD).await.is_ok());
    for absent in ["ghost", "Not A Handle"] {
        assert!(matches!(
            provider.set_password(absent, password(PASSWORD)).await,
            Err(Error::NotFound(_))
        ));
    }

    let listed: Vec<(String, String, Role)> = provider
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|account| (account.subject, account.username.to_string(), account.role))
        .collect();
    let row = |name: &str, role| (name.to_owned(), name.to_owned(), role);
    assert_eq!(
        listed,
        [
            row("adam-b", Role::Admin),
            row("adam1", Role::Member),
            row("eve", Role::Admin),
        ]
    );
}

async fn a_deleted_account_is_gone_and_its_username_free(pool: Pool) {
    let provider = with(pool, &[("adam", Role::Member), ("eve", Role::Member)]).await;
    assert!(provider.delete("adam").await.unwrap());
    assert!(!provider.delete("adam").await.unwrap());
    assert!(!provider.delete("Not A Handle").await.unwrap());
    assert!(matches!(
        verify(&provider, "adam", PASSWORD).await,
        Err(Error::Forbidden)
    ));
    let left: Vec<String> = provider
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|account| account.subject)
        .collect();
    assert_eq!(left, ["eve"]);

    provider
        .create(
            &handle("adam"),
            password("a newcomer's password"),
            Role::Admin,
        )
        .await
        .unwrap();
    assert_eq!(
        verify(&provider, "adam", "a newcomer's password")
            .await
            .unwrap()
            .role,
        Some(Role::Admin)
    );
}

async fn reopening_keeps_the_accounts(pool: Pool) {
    with(pool.clone(), &[("adam", Role::Member)]).await;
    let reopened = LocalProvider::open(pool).await.unwrap();
    assert!(verify(&reopened, "adam", PASSWORD).await.is_ok());
}

async fn every_scenario(databases: &Databases) {
    a_password_provider_named_local(databases.pool().await).await;
    the_account_decides_the_role(databases.pool().await).await;
    wrong_password_and_unknown_user_are_both_forbidden(databases.pool().await).await;
    unknown_usernames_cost_a_full_password_check(databases.pool().await).await;
    an_existing_username_conflicts(databases.pool().await).await;
    short_passwords_are_refused(databases.pool().await).await;
    accounts_are_listed_created_and_reset(databases.pool().await).await;
    a_deleted_account_is_gone_and_its_username_free(databases.pool().await).await;
    reopening_keeps_the_accounts(databases.pool().await).await;
}

#[tokio::test]
async fn on_sqlite() {
    every_scenario(&Databases::sqlite()).await;
}

#[tokio::test]
async fn on_postgres() {
    every_scenario(&Databases::postgres().await).await;
}
