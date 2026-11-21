use secrecy::{ExposeSecret, SecretString};
use uuid::Uuid;

use super::{credential, seed_user};
use crate::domain::UserId;
use crate::ports::{Env, SecretStore, UserRepo};

pub async fn secret_store(users: &dyn UserRepo, store: &dyn SecretStore) {
    let adam = seed_user(users, "adam").await;
    let eve = seed_user(users, "eve").await;
    let (a, b) = (credential("A_KEY"), credential("B_KEY"));
    store
        .put(adam.id, &b, SecretString::from("b-1"))
        .await
        .expect("put");
    store
        .put(adam.id, &a, SecretString::from("a-1"))
        .await
        .expect("put");
    store
        .put(adam.id, &a, SecretString::from("a-2"))
        .await
        .expect("overwrite");
    store
        .put(eve.id, &a, SecretString::from("eve-1"))
        .await
        .expect("put");

    assert_eq!(
        exposed(&store.load_all(adam.id).await.unwrap()),
        ["A_KEY=a-2", "B_KEY=b-1"]
    );
    store.delete(adam.id, &b).await.expect("delete");
    store.delete(adam.id, &b).await.expect("delete again");
    assert_eq!(
        exposed(&store.load_all(adam.id).await.unwrap()),
        ["A_KEY=a-2"]
    );
    assert_eq!(
        exposed(&store.load_all(eve.id).await.unwrap()),
        ["A_KEY=eve-1"]
    );
    assert!(
        store
            .load_all(UserId(Uuid::new_v4()))
            .await
            .unwrap()
            .is_empty()
    );
}

fn exposed(env: &Env) -> Vec<String> {
    env.iter()
        .map(|(name, value)| format!("{name}={}", value.expose_secret()))
        .collect()
}
