use std::collections::BTreeMap;

use ts_rs::{Config, TS, TypeVisitor};

use crate::dto::{
    CatalogDto, ConfirmDeletion, DeleteFromInside, KeyDto, Me, NewKey, OwnedSandbox,
    SandboxDetailsDto, SandboxDto, ScheduledDeletion, UserDto, Wake,
};
use crate::environment_dto::{AttachedDto, EntryChangeDto, EntryDto, NewEntryDto, ValueInput};
use crate::error::ErrorBody;
use crate::provider::ProviderListing;
use corral_core::app::{CreateSandbox, FILE_LIMIT};
use corral_core::domain::{SandboxDefaults, SandboxStatus};

/// TypeScript declarations of the types the API takes and answers with, and of every type they
/// use, for the web UI.
pub struct TypeScript {
    config: Config,
    declarations: BTreeMap<String, String>,
}

impl TypeScript {
    /// The API's own types; providers add the types of their routes.
    #[must_use]
    pub fn api() -> Self {
        let mut types = Self {
            config: Config::new(),
            declarations: BTreeMap::new(),
        };
        types
            .add::<Me>()
            .add::<CatalogDto>()
            .add::<CreateSandbox>()
            .add::<SandboxDefaults>()
            .add::<SandboxDto>()
            .add::<SandboxStatus>()
            .add::<Wake>()
            .add::<ConfirmDeletion>()
            .add::<ScheduledDeletion>()
            .add::<DeleteFromInside>()
            .add::<OwnedSandbox>()
            .add::<SandboxDetailsDto>()
            .add::<NewKey>()
            .add::<KeyDto>()
            .add::<EntryDto>()
            .add::<NewEntryDto>()
            .add::<EntryChangeDto>()
            .add::<ValueInput>()
            .add::<AttachedDto>()
            .add::<UserDto>()
            .add::<ProviderListing>()
            .add::<ErrorBody>()
            .limit(
                "FILE_LIMIT",
                "The most bytes an environment file may hold.",
                FILE_LIMIT,
            );
        types
    }

    pub fn add<T: TS + 'static>(&mut self) -> &mut Self {
        self.visit::<T>();
        self
    }

    /// A limit the API enforces, for forms to check before sending.
    pub fn limit(&mut self, name: &str, docs: &str, value: usize) -> &mut Self {
        self.declarations.insert(
            name.to_owned(),
            format!("/**\n * {docs}\n */\nexport const {name} = {value};"),
        );
        self
    }

    /// One module declaring every type, by name.
    #[must_use]
    pub fn render(&self) -> String {
        let mut module = String::from(
            "// Generated from the Rust API types by ts-rs. Do not edit: regenerate with\n\
             // CORRAL_WRITE_TYPES=1 cargo test -p corral-server web_types\n",
        );
        for declaration in self.declarations.values() {
            module.push('\n');
            for line in declaration.lines() {
                module.push_str(line.trim_end());
                module.push('\n');
            }
        }
        module
    }
}

impl TypeVisitor for TypeScript {
    fn visit<T: TS + 'static + ?Sized>(&mut self) {
        // Only declarable types have an output path; the rest, such as `string`, are inlined.
        if T::output_path().is_none() {
            return;
        }
        let name = T::ident(&self.config);
        if self.declarations.contains_key(&name) {
            return;
        }
        let declaration = format!(
            "{}export {}",
            T::docs().unwrap_or_default(),
            T::decl(&self.config)
        );
        self.declarations.insert(name, declaration);
        T::visit_dependencies(self);
    }
}
