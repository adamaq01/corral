//! How the lobby words its choices, the same on a full screen and on lines.

use corral_core::domain::Shared;

use super::installer::{Installer, Phase, Step};
use crate::api::{EntryKind, ImageChoice, OfferedEntry, SizeChoice};

const GIB: u64 = 1 << 30;
const MIB: u64 = 1 << 20;

pub(crate) const ANOTHER_IMAGE: &str = "Another image";

pub(crate) fn image(image: &ImageChoice) -> String {
    format!("{} ({})", image.label, image.key)
}

pub(crate) fn size(size: &SizeChoice) -> String {
    let (whole, part) = (size.cpu_millis / 1000, size.cpu_millis % 1000);
    let cpus = if part == 0 {
        whole.to_string()
    } else {
        format!("{whole}.{}", format!("{part:03}").trim_end_matches('0'))
    };
    format!(
        "{} ({}): {cpus} CPU{}, {} memory, {} home",
        size.label,
        size.key,
        if size.cpu_millis == 1000 { "" } else { "s" },
        bytes(size.memory_bytes),
        bytes(size.home_bytes)
    )
}

pub(crate) fn bytes(bytes: u64) -> String {
    if bytes >= GIB && bytes.is_multiple_of(GIB) {
        format!("{} GiB", bytes / GIB)
    } else {
        format!("{} MiB", bytes.div_ceil(MIB))
    }
}

/// An entry's kind and what it is, never its value.
pub(crate) fn kind(entry: &OfferedEntry) -> String {
    let mut words = vec![match entry.kind {
        EntryKind::Variable => "variable",
        EntryKind::File => "file",
    }];
    if entry.template {
        words.push("template");
    }
    if entry.secret {
        words.push("secret");
    }
    match entry.shared {
        Some(Shared::Required) => words.push("required"),
        Some(Shared::Offered) => words.push("offered"),
        None => {}
    }
    if entry.apply_to_new && !entry.required() {
        words.push("for new sandboxes");
    }
    words.join(", ")
}

/// A required entry, which no choice leaves out.
pub(crate) fn fixed(entry: &OfferedEntry) -> String {
    format!("= {}  {}", entry.name, kind(entry))
}

pub(crate) fn step(step: &Step) -> &'static str {
    match step {
        Step::Name => "Name",
        Step::Image | Step::Reference => "Image",
        Step::Size => "Size",
        Step::Setup => "Setup",
        Step::Repository(_) => "Repository",
        Step::Summary | Step::Making(_) | Step::Failed(_) => "Summary",
    }
}

/// The image chosen, as the summary shows it.
pub(crate) fn chosen_image(installer: &Installer) -> String {
    match installer.offer.images.get(installer.image) {
        Some(choice) => image(choice),
        None => installer.reference.trim().to_owned(),
    }
}

pub(crate) fn chosen_size(installer: &Installer) -> String {
    installer
        .offer
        .sizes
        .get(installer.size)
        .map_or_else(String::new, size)
}

pub(crate) fn chosen_setup(installer: &Installer) -> String {
    let names: Vec<&str> = installer
        .chosen_entries()
        .map(|entry| entry.name.as_str())
        .collect();
    if names.is_empty() {
        "none".to_owned()
    } else {
        names.join(", ")
    }
}

pub(crate) fn chosen_repository(installer: &Installer) -> String {
    match installer.repository() {
        Some(repository) => match repository.branch() {
            Some(branch) => format!("{} ({branch})", repository.url()),
            None => repository.url().to_owned(),
        },
        None => "none".to_owned(),
    }
}

pub(crate) fn progress(installer: &Installer, phase: Phase) -> String {
    let doing = match phase {
        Phase::Creating => "Creating",
        Phase::Starting => "Starting",
    };
    format!("{doing} {}... {} s", installer.login(), installer.seconds)
}

pub(crate) fn named_images(installer: &Installer) -> String {
    match &installer.offer.named_images {
        Some(named) => format!(
            "An image reference from {}, at most {} to download.",
            named.registries.join(", "),
            bytes(named.max_bytes)
        ),
        None => String::new(),
    }
}

pub(crate) const LEFT: &str = "left the lobby, and nothing was made";

#[cfg(test)]
mod tests {
    use super::{bytes, size};
    use crate::api::SizeChoice;

    #[test]
    fn sizes_read_in_cpus_and_binary_units() {
        let choice = |cpu_millis, memory_bytes| SizeChoice {
            key: "small".to_owned(),
            label: "Small".to_owned(),
            cpu_millis,
            memory_bytes,
            home_bytes: 10 << 30,
        };
        assert_eq!(
            size(&choice(1000, 4 << 30)),
            "Small (small): 1 CPU, 4 GiB memory, 10 GiB home"
        );
        assert_eq!(
            size(&choice(500, 1536 << 20)),
            "Small (small): 0.5 CPUs, 1536 MiB memory, 10 GiB home"
        );
        assert_eq!(bytes(500 << 20), "500 MiB");
    }
}
