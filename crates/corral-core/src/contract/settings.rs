use std::num::NonZeroU64;

use super::seed_user;
use crate::app::{
    Allowance, CatalogChange, CatalogDefaults, ImageOption, LimitsChange, NamedImages, Registry,
    SizePreset, Source,
};
use crate::domain::{ImageKey, SizeKey};
use crate::fakes::{image_option, size_preset};
use crate::ports::{SettingsRepo, UserRepo};

/// Expects a store that holds no settings yet.
pub async fn settings_repo(users: &dyn UserRepo, settings: &dyn SettingsRepo) {
    nothing_is_stored_before_a_write(settings).await;
    the_catalog_round_trips_as_database_entries(settings).await;
    limits_round_trip_and_a_write_replaces_them(settings).await;
    registries_round_trip_and_a_write_replaces_them(settings).await;
    allowances_are_set_replaced_and_cleared(users, settings).await;
}

fn from_config(image: ImageOption) -> ImageOption {
    ImageOption {
        source: Source::Config,
        ..image
    }
}

async fn nothing_is_stored_before_a_write(settings: &dyn SettingsRepo) {
    assert_eq!(settings.catalog().await.expect("read catalog"), None);
    assert_eq!(
        settings.limits().await.expect("read limits"),
        LimitsChange {
            max_sandboxes_per_member: None,
            max_running: None,
        }
    );
    assert_eq!(settings.registries().await.expect("read registries"), []);
}

async fn the_catalog_round_trips_as_database_entries(settings: &dyn SettingsRepo) {
    let written = CatalogChange {
        images: vec![
            from_config(image_option("base", "ghcr.io/acme/base:1")),
            ImageOption {
                description: "With the build tools.".to_owned(),
                enabled: false,
                ..from_config(image_option("tools", "ghcr.io/acme/tools:2"))
            },
        ],
        sizes: vec![
            SizePreset {
                source: Source::Config,
                ..size_preset("small", 1000, 2)
            },
            SizePreset {
                enabled: false,
                ..size_preset("large", 4000, 8)
            },
        ],
        defaults: Some(CatalogDefaults {
            image: ImageKey::parse("base").unwrap(),
            size: SizeKey::parse("small").unwrap(),
        }),
        named_images: Some(NamedImages {
            registries: vec!["ghcr.io/acme".to_owned(), "docker.io".to_owned()],
            max_bytes: NonZeroU64::new(2 << 30).unwrap(),
        }),
    };
    settings.set_catalog(&written).await.expect("set catalog");
    let read = settings
        .catalog()
        .await
        .expect("read catalog")
        .expect("a stored catalog");
    assert_eq!(
        read,
        CatalogChange {
            images: vec![
                image_option("base", "ghcr.io/acme/base:1"),
                ImageOption {
                    description: "With the build tools.".to_owned(),
                    enabled: false,
                    ..image_option("tools", "ghcr.io/acme/tools:2")
                },
            ],
            sizes: vec![
                size_preset("small", 1000, 2),
                SizePreset {
                    enabled: false,
                    ..size_preset("large", 4000, 8)
                },
            ],
            ..written
        }
    );

    let replacing = CatalogChange {
        images: vec![image_option("ubuntu", "ubuntu:24.04")],
        sizes: Vec::new(),
        defaults: None,
        named_images: None,
    };
    settings
        .set_catalog(&replacing)
        .await
        .expect("replace catalog");
    assert_eq!(
        settings.catalog().await.expect("read catalog"),
        Some(replacing)
    );
}

async fn limits_round_trip_and_a_write_replaces_them(settings: &dyn SettingsRepo) {
    for limits in [
        LimitsChange {
            max_sandboxes_per_member: Some(4),
            max_running: Some(2),
        },
        LimitsChange {
            max_sandboxes_per_member: None,
            max_running: Some(8),
        },
    ] {
        settings.set_limits(&limits).await.expect("set limits");
        assert_eq!(settings.limits().await.expect("read limits"), limits);
    }
}

async fn registries_round_trip_and_a_write_replaces_them(settings: &dyn SettingsRepo) {
    let ghcr = Registry {
        registry: "ghcr.io".to_owned(),
        username: Some("corral".to_owned()),
        password_env: Some("CORRAL_GHCR_TOKEN".to_owned()),
        plain_http: false,
        source: Source::Database,
    };
    let lan = Registry {
        registry: "registry.lan:5000".to_owned(),
        username: None,
        password_env: None,
        plain_http: true,
        source: Source::Database,
    };
    settings
        .set_registries(&[
            Registry {
                source: Source::Config,
                ..ghcr.clone()
            },
            lan.clone(),
        ])
        .await
        .expect("set registries");
    assert_eq!(
        settings.registries().await.expect("read registries"),
        [ghcr, lan.clone()]
    );
    settings
        .set_registries(std::slice::from_ref(&lan))
        .await
        .expect("replace registries");
    assert_eq!(settings.registries().await.expect("read registries"), [lan]);
}

async fn allowances_are_set_replaced_and_cleared(
    users: &dyn UserRepo,
    settings: &dyn SettingsRepo,
) {
    let user = seed_user(users, "allowed").await;
    assert_eq!(
        settings.allowance(user.id).await.expect("read allowance"),
        None
    );
    for max_sandboxes in [4, 9] {
        let allowance = Allowance { max_sandboxes };
        settings
            .set_allowance(user.id, Some(&allowance))
            .await
            .expect("set allowance");
        assert_eq!(
            settings.allowance(user.id).await.expect("read allowance"),
            Some(allowance)
        );
    }
    settings
        .set_allowance(user.id, None)
        .await
        .expect("clear allowance");
    assert_eq!(
        settings.allowance(user.id).await.expect("read allowance"),
        None
    );
}
