//! The lobby's choices and the steps through them, whichever way the terminal shows them: keys
//! on a full screen, or answers typed on a line.

use corral_core::Error;
use corral_core::domain::SandboxName;

use super::keys::Key;
use crate::api::{Create, Offer, OfferedEntry, Spec};

const MAX_NAME: usize = 32;
const MAX_REFERENCE: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    /// The login named only its user, who has no sandbox yet.
    Name,
    Image,
    /// An image of the person's own naming, in place of the catalog's.
    Reference,
    Size,
    Setup,
    Summary,
    Making(Phase),
    /// Making refused, for this reason: nothing was made.
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Creating,
    Starting,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Event {
    Key(Key),
    /// A line typed in answer to the step's question.
    Answer(String),
    /// Why the image reference cannot be used, if it cannot.
    ImageChecked(Option<String>),
    /// Made, or why not.
    Made(Result<(), String>),
    /// Seconds into the phase of making.
    Seconds(u64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Effect {
    CheckImage(String),
    Make(Create),
    /// The person leaves, with nothing made.
    Leave,
}

pub(crate) struct Installer {
    pub(crate) offer: Offer,
    pub(crate) step: Step,
    pub(crate) name: String,
    /// The image picked: one of the catalog's, or at its length "Another image".
    pub(crate) image: usize,
    pub(crate) reference: String,
    pub(crate) size: usize,
    pub(crate) chosen: Vec<bool>,
    /// The entry the cursor is on.
    pub(crate) entry: usize,
    /// What is wrong with the last answer, shown with its step.
    pub(crate) problem: Option<String>,
    pub(crate) checking: bool,
    pub(crate) seconds: u64,
}

impl Installer {
    /// Starts from the owner's defaults, with the entries for new sandboxes chosen.
    pub(crate) fn new(offer: Offer) -> Self {
        let image = offer
            .images
            .iter()
            .position(|image| image.key == offer.default_image)
            .unwrap_or(0);
        let size = offer
            .sizes
            .iter()
            .position(|size| size.key == offer.default_size)
            .unwrap_or(0);
        let chosen = offer.entries.iter().map(|e| e.apply_to_new).collect();
        Self {
            step: if offer.name.is_some() {
                Step::Image
            } else {
                Step::Name
            },
            name: offer.name.clone().unwrap_or_default(),
            image,
            reference: String::new(),
            size,
            chosen,
            entry: 0,
            problem: None,
            checking: false,
            seconds: 0,
            offer,
        }
    }

    /// The login of the sandbox being made, as `<name>.<handle>`.
    pub(crate) fn login(&self) -> String {
        format!("{}.{}", self.name, self.offer.handle)
    }

    /// The images to pick from: the catalog's, then "Another image" where users may name one.
    pub(crate) fn image_count(&self) -> usize {
        self.offer.images.len() + usize::from(self.offer.named_images.is_some())
    }

    pub(crate) fn another_image(&self) -> bool {
        self.image == self.offer.images.len()
    }

    /// The image as a summary and a spec name it: a catalog key, or the reference.
    pub(crate) fn image_spec(&self) -> String {
        match self.offer.images.get(self.image) {
            Some(image) => image.key.clone(),
            None => self.reference.trim().to_owned(),
        }
    }

    pub(crate) fn chosen_entries(&self) -> impl Iterator<Item = &OfferedEntry> {
        self.offer
            .entries
            .iter()
            .zip(&self.chosen)
            .filter_map(|(entry, &chosen)| chosen.then_some(entry))
    }

    /// The steps this lobby goes through, for a guide to where the person is.
    pub(crate) fn steps(&self) -> Vec<Step> {
        let mut steps = Vec::new();
        if self.offer.name.is_none() {
            steps.push(Step::Name);
        }
        steps.push(Step::Image);
        if self.another_image() {
            steps.push(Step::Reference);
        }
        steps.push(Step::Size);
        if !self.offer.entries.is_empty() {
            steps.push(Step::Setup);
        }
        steps.push(Step::Summary);
        steps
    }

    pub(crate) fn handle(&mut self, event: Event) -> Option<Effect> {
        match event {
            Event::Key(key) => self.key(key),
            Event::Answer(answer) => self.answer(answer.trim()),
            Event::ImageChecked(problem) => {
                self.checking = false;
                self.problem = problem;
                if self.problem.is_none() && self.step == Step::Reference {
                    self.step = Step::Size;
                }
                None
            }
            Event::Made(Ok(())) => {
                self.step = Step::Making(Phase::Starting);
                self.seconds = 0;
                None
            }
            Event::Made(Err(reason)) => {
                self.step = Step::Failed(reason);
                None
            }
            Event::Seconds(seconds) => {
                self.seconds = seconds;
                None
            }
        }
    }

    fn key(&mut self, key: Key) -> Option<Effect> {
        if matches!(self.step, Step::Making(_)) {
            return None;
        }
        if matches!(key, Key::Esc | Key::Interrupt) {
            return Some(Effect::Leave);
        }
        if self.checking {
            return None;
        }
        match (&self.step, key) {
            (Step::Name, key) => {
                if self.edit(key, MAX_NAME, |installer| &mut installer.name) {
                    return None;
                }
                match key {
                    Key::Enter => self.submit_name(),
                    _ => None,
                }
            }
            (Step::Reference, Key::Up) => {
                self.back();
                None
            }
            (Step::Reference, key) => {
                if self.edit(key, MAX_REFERENCE, |installer| &mut installer.reference) {
                    return None;
                }
                match key {
                    Key::Enter => self.submit_reference(),
                    _ => None,
                }
            }
            (Step::Failed(_), Key::Enter | Key::Left | Key::Backspace) => {
                self.step = Step::Summary;
                None
            }
            (_, Key::Char('q') | Key::End) => Some(Effect::Leave),
            (_, Key::Up | Key::Char('k')) => {
                self.move_cursor(false);
                None
            }
            (_, Key::Down | Key::Char('j') | Key::Tab) => {
                self.move_cursor(true);
                None
            }
            (Step::Setup, Key::Char(' ' | 'x')) => {
                if let Some(chosen) = self.chosen.get_mut(self.entry) {
                    *chosen = !*chosen;
                }
                None
            }
            (_, Key::Left | Key::Backspace) => {
                self.back();
                None
            }
            (_, Key::Enter | Key::Right) => self.next(),
            _ => None,
        }
    }

    /// Types `key` into a text field, saying whether it was a key that edits.
    fn edit(&mut self, key: Key, max: usize, field: impl Fn(&mut Self) -> &mut String) -> bool {
        match key {
            Key::Char(c) => {
                let text = field(self);
                if text.chars().count() < max {
                    text.push(c);
                }
            }
            Key::Backspace => {
                field(self).pop();
            }
            _ => return false,
        }
        self.problem = None;
        true
    }

    fn move_cursor(&mut self, down: bool) {
        let (cursor, count) = match self.step {
            Step::Image => (&mut self.image, self.offer.images.len()),
            Step::Size => (&mut self.size, self.offer.sizes.len()),
            Step::Setup => (&mut self.entry, self.offer.entries.len()),
            _ => return,
        };
        let count = if self.step == Step::Image {
            count + usize::from(self.offer.named_images.is_some())
        } else {
            count
        };
        if down {
            *cursor = (*cursor + 1).min(count.saturating_sub(1));
        } else {
            *cursor = cursor.saturating_sub(1);
        }
    }

    fn next(&mut self) -> Option<Effect> {
        self.problem = None;
        self.step = match self.step {
            Step::Image if self.another_image() => Step::Reference,
            Step::Image => Step::Size,
            Step::Size if self.offer.entries.is_empty() => Step::Summary,
            Step::Size => Step::Setup,
            Step::Setup => Step::Summary,
            Step::Summary => return Some(self.make()),
            _ => return None,
        };
        None
    }

    fn back(&mut self) {
        self.problem = None;
        self.step = match self.step {
            Step::Image if self.offer.name.is_none() => Step::Name,
            Step::Size if self.another_image() => Step::Reference,
            Step::Reference | Step::Size => Step::Image,
            Step::Setup => Step::Size,
            Step::Summary if self.offer.entries.is_empty() => Step::Size,
            Step::Summary => Step::Setup,
            ref step => step.clone(),
        };
    }

    fn submit_name(&mut self) -> Option<Effect> {
        match SandboxName::parse(self.name.trim()) {
            Ok(name) => {
                self.name = name.to_string();
                self.problem = None;
                self.step = Step::Image;
            }
            Err(Error::Invalid(why)) => self.problem = Some(why),
            Err(error) => self.problem = Some(error.to_string()),
        }
        None
    }

    fn submit_reference(&mut self) -> Option<Effect> {
        let reference = self.reference.trim();
        if reference.is_empty() {
            self.problem = Some("type an image reference, such as ghcr.io/acme/tools:1".to_owned());
            return None;
        }
        self.checking = true;
        self.problem = None;
        Some(Effect::CheckImage(reference.to_owned()))
    }

    fn make(&mut self) -> Effect {
        self.step = Step::Making(Phase::Creating);
        self.seconds = 0;
        let spec = Spec {
            image: Some(self.image_spec()),
            size: self.offer.sizes.get(self.size).map(|size| size.key.clone()),
            setup: Some(self.chosen_entries().map(|e| e.id.clone()).collect()),
        };
        Effect::Make(Create {
            username: self.login(),
            spec,
        })
    }

    /// Takes a typed answer to the step's question; an empty one keeps what is there.
    fn answer(&mut self, answer: &str) -> Option<Effect> {
        match self.step {
            Step::Name => {
                if !answer.is_empty() {
                    answer.clone_into(&mut self.name);
                }
                self.submit_name()
            }
            Step::Image => {
                let count = self.image_count();
                self.pick(answer, count).map(|image| self.image = image)?;
                self.next()
            }
            Step::Reference => {
                if !answer.is_empty() {
                    answer.clone_into(&mut self.reference);
                }
                self.submit_reference()
            }
            Step::Size => {
                let count = self.offer.sizes.len();
                self.pick(answer, count).map(|size| self.size = size)?;
                self.next()
            }
            Step::Setup => {
                self.choose(answer)?;
                self.next()
            }
            Step::Summary => match answer.to_ascii_lowercase().as_str() {
                "" | "y" | "yes" => Some(self.make()),
                "n" | "no" => {
                    self.restart();
                    None
                }
                _ => {
                    self.problem = Some("answer y to create it, or n to change it".to_owned());
                    None
                }
            },
            Step::Failed(_) => {
                self.restart();
                None
            }
            Step::Making(_) => None,
        }
    }

    /// The option an answer numbers from 1, or the one picked for an empty answer.
    fn pick(&mut self, answer: &str, count: usize) -> Option<usize> {
        let picked = match self.step {
            Step::Image => self.image,
            _ => self.size,
        };
        if answer.is_empty() {
            return Some(picked);
        }
        match answer.parse::<usize>() {
            Ok(n) if (1..=count).contains(&n) => Some(n - 1),
            _ => {
                self.problem = Some(format!("answer with a number from 1 to {count}"));
                None
            }
        }
    }

    /// Chooses exactly the entries an answer numbers, or none for `none`.
    fn choose(&mut self, answer: &str) -> Option<()> {
        if answer.is_empty() {
            return Some(());
        }
        let count = self.offer.entries.len();
        let mut chosen = vec![false; count];
        if answer != "none" {
            for part in answer.split([' ', ',']).filter(|part| !part.is_empty()) {
                match part.parse::<usize>() {
                    Ok(n) if (1..=count).contains(&n) => chosen[n - 1] = true,
                    _ => {
                        self.problem =
                            Some(format!("answer with numbers from 1 to {count}, or none"));
                        return None;
                    }
                }
            }
        }
        self.chosen = chosen;
        Some(())
    }

    /// Back to the first question, every choice kept.
    fn restart(&mut self) {
        self.problem = None;
        self.step = if self.offer.name.is_none() {
            Step::Name
        } else {
            Step::Image
        };
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{Effect, Event, Installer, Phase, Step};
    use crate::api::{
        Create, EntryKind, ImageChoice, NamedImages, Offer, OfferedEntry, SizeChoice, Spec,
    };
    use crate::lobby::keys::Key;

    pub(crate) fn offer(name: Option<&str>) -> Offer {
        let image = |key: &str, label: &str| ImageChoice {
            key: key.to_owned(),
            label: label.to_owned(),
        };
        let size = |key: &str, label: &str, cpu_millis: u64, gib: u64| SizeChoice {
            key: key.to_owned(),
            label: label.to_owned(),
            cpu_millis,
            memory_bytes: gib << 30,
            home_bytes: 10 << 30,
        };
        let entry = |id: &str, kind, name: &str, secret, apply_to_new| OfferedEntry {
            id: id.to_owned(),
            kind,
            name: name.to_owned(),
            secret,
            template: false,
            apply_to_new,
        };
        Offer {
            handle: "adam".to_owned(),
            name: name.map(str::to_owned),
            images: vec![image("base", "Base"), image("tools", "Tools")],
            named_images: Some(NamedImages {
                registries: vec!["ghcr.io/acme".to_owned()],
                max_bytes: 500 << 20,
            }),
            sizes: vec![
                size("small", "Small", 1000, 2),
                size("large", "Large", 4000, 8),
            ],
            default_image: "tools".to_owned(),
            default_size: "small".to_owned(),
            entries: vec![
                entry("id-1", EntryKind::Variable, "GH_TOKEN", true, true),
                entry("id-2", EntryKind::File, ".gitconfig", false, false),
            ],
        }
    }

    fn keys(installer: &mut Installer, keys: &[Key]) -> Vec<Effect> {
        keys.iter()
            .filter_map(|&key| installer.handle(Event::Key(key)))
            .collect()
    }

    fn typed(text: &str) -> Vec<Key> {
        text.chars().map(Key::Char).collect()
    }

    fn make(image: &str, size: &str, setup: &[&str]) -> Effect {
        Effect::Make(Create {
            username: "new.adam".to_owned(),
            spec: Spec {
                image: Some(image.to_owned()),
                size: Some(size.to_owned()),
                setup: Some(setup.iter().map(|&id| id.to_owned()).collect()),
            },
        })
    }

    #[test]
    fn the_defaults_and_the_entries_for_new_sandboxes_come_chosen() {
        let mut installer = Installer::new(offer(Some("new")));
        assert_eq!(installer.step, Step::Image);
        let made = keys(
            &mut installer,
            &[Key::Enter, Key::Enter, Key::Enter, Key::Enter],
        );
        assert_eq!(made, [make("tools", "small", &["id-1"])]);
        assert_eq!(installer.step, Step::Making(Phase::Creating));
    }

    #[test]
    fn the_person_picks_a_size_and_entries_then_goes_back_and_forth() {
        let mut installer = Installer::new(offer(Some("new")));
        let effects = keys(
            &mut installer,
            &[
                Key::Up,
                Key::Enter,
                Key::Down,
                Key::Down,
                Key::Enter,
                Key::Char(' '),
                Key::Down,
                Key::Char(' '),
                Key::Enter,
                Key::Left,
                Key::Left,
                Key::Up,
                Key::Enter,
                Key::Enter,
            ],
        );
        assert_eq!(effects, Vec::<Effect>::new());
        assert_eq!(installer.step, Step::Summary);
        assert_eq!(
            keys(&mut installer, &[Key::Enter]),
            [make("base", "small", &["id-2"])]
        );
    }

    #[test]
    fn esc_or_q_leaves_at_any_step_before_making_and_q_is_typed_in_a_field() {
        for steps in [0, 1, 2, 3] {
            let mut installer = Installer::new(offer(Some("new")));
            keys(&mut installer, &vec![Key::Enter; steps]);
            assert_eq!(keys(&mut installer, &[Key::Char('q')]), [Effect::Leave]);
            let mut installer = Installer::new(offer(Some("new")));
            keys(&mut installer, &vec![Key::Enter; steps]);
            assert_eq!(keys(&mut installer, &[Key::Esc]), [Effect::Leave]);
        }
        let mut installer = Installer::new(offer(None));
        assert_eq!(keys(&mut installer, &typed("q")), []);
        assert_eq!(installer.name, "q");
        assert_eq!(keys(&mut installer, &[Key::Interrupt]), [Effect::Leave]);

        let mut installer = Installer::new(offer(Some("new")));
        keys(&mut installer, &[Key::Enter; 4]);
        assert_eq!(keys(&mut installer, &[Key::Esc, Key::Char('q')]), []);
    }

    #[test]
    fn a_bare_login_names_its_sandbox_first_and_a_bad_name_says_why() {
        let mut installer = Installer::new(offer(None));
        assert_eq!(installer.step, Step::Name);
        keys(&mut installer, &typed("Bad_name"));
        keys(&mut installer, &[Key::Enter]);
        assert_eq!(installer.step, Step::Name);
        assert_eq!(
            installer.problem.as_deref(),
            Some("sandbox name \"Bad_name\" must match [a-z0-9-]{1,32}")
        );
        keys(&mut installer, &[Key::Backspace; 8]);
        keys(&mut installer, &typed("new"));
        assert_eq!(installer.problem, None);
        keys(&mut installer, &[Key::Enter]);
        assert_eq!(installer.step, Step::Image);
        keys(&mut installer, &[Key::Left]);
        assert_eq!(installer.step, Step::Name);
        assert_eq!(
            keys(&mut installer, &[Key::Enter; 5]),
            [make("tools", "small", &["id-1"])]
        );
    }

    #[test]
    fn another_image_is_checked_before_the_size_and_its_problem_shows_until_it_changes() {
        let mut installer = Installer::new(offer(Some("new")));
        keys(
            &mut installer,
            &[Key::Down, Key::Down, Key::Down, Key::Enter],
        );
        assert_eq!(installer.step, Step::Reference);
        assert_eq!(keys(&mut installer, &[Key::Enter]), []);
        assert!(installer.problem.is_some());
        let checked = keys(&mut installer, &typed("debian:13"));
        assert_eq!(checked, Vec::<Effect>::new());
        assert_eq!(
            keys(&mut installer, &[Key::Enter]),
            [Effect::CheckImage("debian:13".to_owned())]
        );
        assert_eq!(keys(&mut installer, &[Key::Enter, Key::Char('x')]), []);
        installer.handle(Event::ImageChecked(Some(
            "not from ghcr.io/acme".to_owned(),
        )));
        assert_eq!(
            (&installer.step, installer.problem.as_deref()),
            (&Step::Reference, Some("not from ghcr.io/acme"))
        );
        keys(&mut installer, &[Key::Backspace; 9]);
        keys(&mut installer, &typed("ghcr.io/acme/tools:1"));
        assert_eq!(installer.problem, None);
        keys(&mut installer, &[Key::Enter]);
        installer.handle(Event::ImageChecked(None));
        assert_eq!(installer.step, Step::Size);
        keys(&mut installer, &[Key::Left]);
        assert_eq!(installer.step, Step::Reference);
        keys(&mut installer, &[Key::Up]);
        assert_eq!(installer.step, Step::Image);
    }

    #[test]
    fn a_refused_make_shows_why_and_goes_back_to_the_summary() {
        let mut installer = Installer::new(offer(Some("new")));
        keys(&mut installer, &[Key::Enter; 4]);
        installer.handle(Event::Made(Err("at most 3 sandboxes per user".to_owned())));
        assert_eq!(
            installer.step,
            Step::Failed("at most 3 sandboxes per user".to_owned())
        );
        keys(&mut installer, &[Key::Enter]);
        assert_eq!(installer.step, Step::Summary);
        assert_eq!(keys(&mut installer, &[Key::Enter]).len(), 1);
        installer.handle(Event::Made(Ok(())));
        installer.handle(Event::Seconds(4));
        assert_eq!(
            (&installer.step, installer.seconds),
            (&Step::Making(Phase::Starting), 4)
        );
    }

    #[test]
    fn typed_answers_pick_by_number_and_an_empty_one_keeps_the_choice() {
        let mut installer = Installer::new(offer(None));
        let answer = |installer: &mut Installer, text: &str| {
            installer.handle(Event::Answer(text.to_owned()))
        };
        assert_eq!(answer(&mut installer, " new "), None);
        assert_eq!(answer(&mut installer, "9"), None);
        assert_eq!(
            installer.problem.as_deref(),
            Some("answer with a number from 1 to 3")
        );
        assert_eq!(answer(&mut installer, ""), None);
        assert_eq!(answer(&mut installer, "2"), None);
        assert_eq!(installer.step, Step::Setup);
        assert_eq!(answer(&mut installer, "2 x"), None);
        assert!(installer.problem.is_some());
        assert_eq!(answer(&mut installer, "2"), None);
        assert_eq!(answer(&mut installer, "n"), None);
        assert_eq!(installer.step, Step::Name);
        for text in ["", "", "", ""] {
            assert_eq!(answer(&mut installer, text), None);
        }
        assert_eq!(
            answer(&mut installer, "yes"),
            Some(make("tools", "large", &["id-2"]))
        );
        installer.handle(Event::Made(Err("no".to_owned())));
        answer(&mut installer, "");
        assert_eq!(installer.step, Step::Name);
        assert_eq!(installer.name, "new");
    }
}
