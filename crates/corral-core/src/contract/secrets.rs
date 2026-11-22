use secrecy::SecretString;
use uuid::Uuid;

use crate::domain::UserId;
use crate::fakes::{credential, exposed};
use crate::ports::SecretStore;

fn user() -> UserId {
    UserId(Uuid::new_v4())
}

pub async fn secret_store(store: &dyn SecretStore) {
    values_round_trip(store).await;
    names_list_in_byte_order(store).await;
    remove_reports_whether_it_existed(store).await;
    users_are_isolated(store).await;
}

async fn values_round_trip(store: &dyn SecretStore) {
    let adam = user();
    let (a, b) = (credential("A_KEY"), credential("B_KEY"));
    for (name, value) in [(&b, "b-1"), (&a, "a-1"), (&a, "a-2")] {
        store
            .set(adam, name, SecretString::from(value))
            .await
            .expect("set");
    }
    let env = store.load(adam).await.expect("load");
    assert_eq!(exposed(&env), ["A_KEY=a-2", "B_KEY=b-1"]);
    let debug = format!("{env:?}");
    assert!(
        debug.contains("A_KEY") && !debug.contains("a-2") && !debug.contains("b-1"),
        "{debug}"
    );
}

async fn names_list_in_byte_order(store: &dyn SecretStore) {
    let owner = user();
    let mut names: Vec<_> = ["A_A", "AB", "A1", "A_"].map(credential).into();
    for name in &names {
        store
            .set(owner, name, SecretString::from("x"))
            .await
            .expect("set");
    }
    names.sort();
    assert_eq!(store.names(owner).await.expect("names"), names);
}

async fn remove_reports_whether_it_existed(store: &dyn SecretStore) {
    let adam = user();
    let (a, b) = (credential("A_KEY"), credential("B_KEY"));
    for name in [&a, &b] {
        store
            .set(adam, name, SecretString::from("x"))
            .await
            .expect("set");
    }
    assert!(store.remove(adam, &b).await.expect("remove"));
    assert!(!store.remove(adam, &b).await.expect("remove again"));
    assert_eq!(store.names(adam).await.unwrap(), std::slice::from_ref(&a));
    assert_eq!(exposed(&store.load(adam).await.unwrap()), ["A_KEY=x"]);

    assert!(store.remove(adam, &a).await.expect("remove the last"));
    assert_eq!(store.names(adam).await.unwrap(), []);
    assert!(store.load(adam).await.unwrap().is_empty());
    assert!(!store.remove(user(), &a).await.unwrap());
}

async fn users_are_isolated(store: &dyn SecretStore) {
    let adam = user();
    let eve = user();
    let (a, b) = (credential("A_KEY"), credential("B_KEY"));
    store
        .set(adam, &a, SecretString::from("adam-a"))
        .await
        .expect("set");
    store
        .set(adam, &b, SecretString::from("adam-b"))
        .await
        .expect("set");
    store
        .set(eve, &a, SecretString::from("eve-a"))
        .await
        .expect("set");

    assert!(!store.remove(eve, &b).await.unwrap());
    assert_eq!(store.names(eve).await.unwrap(), std::slice::from_ref(&a));
    assert_eq!(exposed(&store.load(eve).await.unwrap()), ["A_KEY=eve-a"]);
    assert_eq!(
        exposed(&store.load(adam).await.unwrap()),
        ["A_KEY=adam-a", "B_KEY=adam-b"]
    );
    let stranger = user();
    assert_eq!(store.names(stranger).await.unwrap(), []);
    assert!(store.load(stranger).await.unwrap().is_empty());
}
