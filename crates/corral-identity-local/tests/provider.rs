use std::sync::Arc;
use std::time::{Duration, Instant};

use corral_core::Error;
use corral_core::app::Accounts;
use corral_core::domain::{Admin, Handle, Role, User};
use corral_core::fakes::{MemoryLocalAccountRepo, MemoryUserRepo};
use corral_core::ports::{
    AccountAdmin, Answer, Challenge, Identity, IdentityProvider, Method, PendingLogin,
};
use corral_identity_local::LocalProvider;
use secrecy::SecretString;

const PASSWORD: &str = "correct horse battery";

struct World {
    provider: LocalProvider,
    accounts: Accounts,
    root: User,
}

impl World {
    async fn with(accounts: &[(&str, Role)]) -> Self {
        let users = Arc::new(MemoryUserRepo::default());
        let provider =
            LocalProvider::new(Arc::new(MemoryLocalAccountRepo::default()), users.clone()).unwrap();
        let logins = Accounts::new(users);
        provider
            .add(&handle("root"), password(PASSWORD), Role::Admin)
            .await
            .unwrap();
        let root = logins
            .login(
                provider.id(),
                provider.verify(answer("root", PASSWORD)).await.unwrap(),
            )
            .await
            .unwrap();
        for &(username, role) in accounts {
            provider
                .add(&handle(username), password(PASSWORD), role)
                .await
                .unwrap();
        }
        Self {
            provider,
            accounts: logins,
            root,
        }
    }

    fn admin(&self) -> Admin {
        Admin::check(&self.root).unwrap()
    }

    fn admin_api(&self) -> &dyn AccountAdmin {
        self.provider
            .accounts()
            .expect("local accounts are managed")
    }

    async fn verify(&self, username: &str, attempt: &str) -> Result<Identity, Error> {
        self.provider.verify(answer(username, attempt)).await
    }
}

fn handle(value: &str) -> Handle {
    Handle::parse(value).unwrap()
}

fn password(value: &str) -> SecretString {
    SecretString::from(value)
}

fn answer(username: &str, attempt: &str) -> Answer {
    Answer::Password {
        username: username.to_owned(),
        password: password(attempt),
    }
}

#[tokio::test]
async fn a_password_provider_named_local() {
    let w = World::with(&[]).await;
    assert_eq!(w.provider.id(), "local");
    assert_eq!(w.provider.method(), Method::Password);
    assert_eq!(w.provider.challenge().await.unwrap(), Challenge::Password);
    let redirect = Answer::Redirect {
        pending: PendingLogin("x".to_owned()),
        params: [("code".to_owned(), "x".to_owned())].into(),
    };
    assert!(matches!(
        w.provider.verify(redirect).await,
        Err(Error::Invalid(_))
    ));
}

#[tokio::test]
async fn the_account_decides_the_role() {
    let w = World::with(&[("adam", Role::Member)]).await;
    assert_eq!(
        w.verify("adam", PASSWORD).await.unwrap(),
        Identity {
            subject: "adam".to_owned(),
            username: "adam".to_owned(),
            email: None,
            role: Some(Role::Member),
        }
    );
    assert_eq!(w.root.role, Role::Admin);
    assert_eq!(w.root.handle.as_str(), "root");
}

#[tokio::test]
async fn wrong_password_and_unknown_user_are_both_forbidden() {
    let w = World::with(&[("adam", Role::Member)]).await;
    for (username, attempt) in [
        ("adam", "wrong horse battery"),
        ("nobody", PASSWORD),
        ("Not A Handle", PASSWORD),
        ("Adam", PASSWORD),
    ] {
        let result = w.verify(username, attempt).await;
        assert!(
            matches!(result, Err(Error::Forbidden)),
            "{username}: {result:?}"
        );
    }
}

#[tokio::test]
async fn unknown_usernames_cost_a_full_password_check() {
    let w = World::with(&[("adam", Role::Member)]).await;
    let fastest = |username: &'static str| {
        let w = &w;
        async move {
            let mut best = Duration::MAX;
            for _ in 0..3 {
                let started = Instant::now();
                let _ = w.verify(username, "wrong horse battery").await;
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

#[tokio::test]
async fn an_existing_username_conflicts() {
    let w = World::with(&[("adam", Role::Member)]).await;
    assert!(matches!(
        w.admin_api()
            .create(
                &w.admin(),
                &handle("adam"),
                password("another password"),
                Role::Admin
            )
            .await,
        Err(Error::Conflict(_))
    ));
    assert_eq!(
        w.verify("adam", PASSWORD).await.unwrap().role,
        Some(Role::Member)
    );
}

#[tokio::test]
async fn short_passwords_are_refused() {
    let w = World::with(&[("adam", Role::Member)]).await;
    let short = password("elevenchars");
    assert!(matches!(
        w.provider
            .add(&handle("eve"), short.clone(), Role::Member)
            .await,
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        w.admin_api()
            .set_password(&w.admin(), &handle("adam"), short)
            .await,
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        w.verify("eve", "elevenchars").await,
        Err(Error::Forbidden)
    ));
    assert!(w.verify("adam", PASSWORD).await.is_ok());
}

#[tokio::test]
async fn admins_list_create_and_reset_passwords() {
    let w = World::with(&[("adam", Role::Member)]).await;
    let admin = w.admin();
    w.admin_api()
        .create(&admin, &handle("eve"), password(PASSWORD), Role::Admin)
        .await
        .unwrap();
    w.admin_api()
        .set_password(&admin, &handle("adam"), password("a brand new password"))
        .await
        .unwrap();
    assert!(matches!(
        w.verify("adam", PASSWORD).await,
        Err(Error::Forbidden)
    ));
    assert!(w.verify("adam", "a brand new password").await.is_ok());
    assert_eq!(
        w.verify("eve", PASSWORD).await.unwrap().role,
        Some(Role::Admin)
    );
    assert!(matches!(
        w.admin_api()
            .set_password(&admin, &handle("ghost"), password(PASSWORD))
            .await,
        Err(Error::NotFound(_))
    ));

    let listed: Vec<(String, Role)> = w
        .admin_api()
        .list(&admin)
        .await
        .unwrap()
        .into_iter()
        .map(|account| (account.username.to_string(), account.role))
        .collect();
    assert_eq!(
        listed,
        [
            ("adam".to_owned(), Role::Member),
            ("eve".to_owned(), Role::Admin),
            ("root".to_owned(), Role::Admin),
        ]
    );
}

#[tokio::test]
async fn deleting_an_account_disables_its_user() {
    let w = World::with(&[("adam", Role::Member), ("never", Role::Member)]).await;
    let admin = w.admin();
    let adam = w
        .accounts
        .login(w.provider.id(), w.verify("adam", PASSWORD).await.unwrap())
        .await
        .unwrap();

    w.admin_api().delete(&admin, &handle("adam")).await.unwrap();
    assert!(matches!(
        w.accounts.require_active(adam.id).await,
        Err(Error::Forbidden)
    ));
    assert!(matches!(
        w.verify("adam", PASSWORD).await,
        Err(Error::Forbidden)
    ));
    assert!(matches!(
        w.admin_api().delete(&admin, &handle("adam")).await,
        Err(Error::NotFound(_))
    ));

    w.admin_api()
        .delete(&admin, &handle("never"))
        .await
        .unwrap();
    assert!(w.accounts.require_active(w.root.id).await.is_ok());
    let left: Vec<String> = w
        .admin_api()
        .list(&admin)
        .await
        .unwrap()
        .into_iter()
        .map(|account| account.username.to_string())
        .collect();
    assert_eq!(left, ["root"]);
}

#[tokio::test]
async fn a_deleted_username_that_logged_in_is_never_reused() {
    let w = World::with(&[("adam", Role::Member), ("never", Role::Member)]).await;
    let admin = w.admin();
    w.accounts
        .login(w.provider.id(), w.verify("adam", PASSWORD).await.unwrap())
        .await
        .unwrap();
    w.admin_api().delete(&admin, &handle("adam")).await.unwrap();
    w.admin_api()
        .delete(&admin, &handle("never"))
        .await
        .unwrap();

    let newcomer = password("a newcomer's password");
    assert!(matches!(
        w.provider
            .add(&handle("adam"), newcomer.clone(), Role::Admin)
            .await,
        Err(Error::Conflict(_))
    ));
    assert!(matches!(
        w.admin_api()
            .create(&admin, &handle("adam"), newcomer, Role::Admin)
            .await,
        Err(Error::Conflict(_))
    ));
    assert!(matches!(
        w.verify("adam", "a newcomer's password").await,
        Err(Error::Forbidden)
    ));

    w.admin_api()
        .create(&admin, &handle("never"), password(PASSWORD), Role::Member)
        .await
        .unwrap();
    assert!(w.verify("never", PASSWORD).await.is_ok());
}
