use sqlx::migrate::Migrator;

/// A module's own migration set, one directory per driver, recorded in a table of its own
/// (`_<module>_migrations`), so modules number their migrations independently and a database
/// holds any mix of them.
pub struct Migrations {
    pub(crate) module: &'static str,
    sqlite: Migrator,
    postgres: Migrator,
}

impl Migrations {
    /// `module` is a plain SQL identifier, unique among the modules of one database.
    #[must_use]
    pub fn new(module: &'static str, sqlite: Migrator, postgres: Migrator) -> Self {
        Self {
            module,
            sqlite,
            postgres,
        }
    }

    pub(crate) fn sqlite(self) -> Migrator {
        self.tracked(|m| m.sqlite)
    }

    pub(crate) fn postgres(self) -> Migrator {
        self.tracked(|m| m.postgres)
    }

    fn tracked(self, driver: impl FnOnce(Self) -> Migrator) -> Migrator {
        let table = format!("_{}_migrations", self.module);
        let mut migrator = driver(self);
        migrator.dangerous_set_table_name(table);
        migrator
    }
}
