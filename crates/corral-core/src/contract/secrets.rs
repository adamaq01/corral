use secrecy::SecretString;
use uuid::Uuid;

use super::seed_user;
use crate::domain::UserId;
use crate::fakes::{credential, exposed};
use crate::ports::{SecretStore, UserRepo};

pub async fn secret_store(users: &dyn UserRepo, store: &dyn SecretStore) {
    values_round_trip(users, store).await;
    names_list_in_byte_order(users, store).await;
    remove_reports_whether_it_existed(users, store).await;
    users_are_isolated(users, store).await;
}

async fn values_round_trip(users: &dyn UserRepo, store: &dyn SecretStore) {
    let adam = seed_user(users, "adam").await;
    let (a, b) = (credential("A_KEY"), credential("B_KEY"));
    for (name, value) in [(&b, "b-1"), (&a, "a-1"), (&a, "a-2")] {
        store
            .set(adam.id, name, SecretString::from(value))
            .await
            .expect("set");
    }
    let env = store.load(adam.id).await.expect("load");
    assert_eq!(exposed(&env), ["A_KEY=a-2", "B_KEY=b-1"]);
    let debug = format!("{env:?}");
    assert!(
        debug.contains("A_KEY") && !debug.contains("a-2") && !debug.contains("b-1"),
        "{debug}"
    );
}

async fn names_list_in_byte_order(users: &dyn UserRepo, store: &dyn SecretStore) {
    let user = seed_user(users, "collation").await;
    let mut names: Vec<_> = ["A_A", "AB", "A1", "A_"].map(credential).into();
    for name in &names {
        store
            .set(user.id, name, SecretString::from("x"))
            .await
            .expect("set");
    }
    names.sort();
    assert_eq!(store.names(user.id).await.expect("names"), names);
}

async fn remove_reports_whether_it_existed(users: &dyn UserRepo, store: &dyn SecretStore) {
    let adam = seed_user(users, "adam").await;
    let (a, b) = (credential("A_KEY"), credential("B_KEY"));
    for name in [&a, &b] {
        store
            .set(adam.id, name, SecretString::from("x"))
            .await
            .expect("set");
    }
    assert!(store.remove(adam.id, &b).await.expect("remove"));
    assert!(!store.remove(adam.id, &b).await.expect("remove again"));
    assert_eq!(
        store.names(adam.id).await.unwrap(),
        std::slice::from_ref(&a)
    );
    assert_eq!(exposed(&store.load(adam.id).await.unwrap()), ["A_KEY=x"]);

    assert!(store.remove(adam.id, &a).await.expect("remove the last"));
    assert_eq!(store.names(adam.id).await.unwrap(), []);
    assert!(store.load(adam.id).await.unwrap().is_empty());
    assert!(!store.remove(UserId(Uuid::new_v4()), &a).await.unwrap());
}

async fn users_are_isolated(users: &dyn UserRepo, store: &dyn SecretStore) {
    let adam = seed_user(users, "adam").await;
    let eve = seed_user(users, "eve").await;
    let (a, b) = (credential("A_KEY"), credential("B_KEY"));
    store
        .set(adam.id, &a, SecretString::from("adam-a"))
        .await
        .expect("set");
    store
        .set(adam.id, &b, SecretString::from("adam-b"))
        .await
        .expect("set");
    store
        .set(eve.id, &a, SecretString::from("eve-a"))
        .await
        .expect("set");

    assert!(!store.remove(eve.id, &b).await.unwrap());
    assert_eq!(store.names(eve.id).await.unwrap(), std::slice::from_ref(&a));
    assert_eq!(exposed(&store.load(eve.id).await.unwrap()), ["A_KEY=eve-a"]);
    assert_eq!(
        exposed(&store.load(adam.id).await.unwrap()),
        ["A_KEY=adam-a", "B_KEY=adam-b"]
    );
    let stranger = UserId(Uuid::new_v4());
    assert_eq!(store.names(stranger).await.unwrap(), []);
    assert!(store.load(stranger).await.unwrap().is_empty());
}
