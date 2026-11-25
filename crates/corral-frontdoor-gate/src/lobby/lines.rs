//! The lobby as questions on lines, for a terminal that cannot draw a full screen, such as one
//! with `TERM=dumb`. The client's terminal is raw, so the lobby echoes what is typed itself.

use std::fmt::Write as _;

use super::View;
use super::installer::{Event, Installer, Step};
use super::keys::{Key, keys};
use super::words;

pub(crate) struct Lines {
    line: String,
    shown: Option<Shown>,
}

/// What the last question printed was about.
#[derive(Clone, PartialEq, Eq)]
struct Shown {
    step: Step,
    problem: Option<String>,
    checking: bool,
}

impl Lines {
    pub(crate) fn new() -> Self {
        Self {
            line: String::new(),
            shown: None,
        }
    }
}

impl View for Lines {
    fn open(&mut self, installer: &Installer) -> String {
        let intro = match &installer.offer.name {
            Some(_) => format!(
                "Corral: there is no sandbox {} yet. Answer each question to make it, or press \
                 Esc to leave.\r\n",
                installer.login()
            ),
            None => format!(
                "Corral: {} has no sandbox yet. Answer each question to make one, or press Esc \
                 to leave.\r\n",
                installer.offer.handle
            ),
        };
        intro + &self.show(installer)
    }

    fn input(&mut self, data: &[u8], echo: &mut String) -> Vec<Event> {
        let mut events = Vec::new();
        for key in keys(data) {
            match key {
                Key::Char(c) => {
                    self.line.push(c);
                    echo.push(c);
                }
                Key::Backspace => {
                    if self.line.pop().is_some() {
                        echo.push_str("\x08 \x08");
                    }
                }
                Key::Enter => {
                    echo.push_str("\r\n");
                    events.push(Event::Answer(std::mem::take(&mut self.line)));
                }
                Key::Esc | Key::Interrupt => events.push(Event::Key(Key::Esc)),
                Key::End if self.line.is_empty() => events.push(Event::Key(Key::Esc)),
                _ => {}
            }
        }
        events
    }

    fn show(&mut self, installer: &Installer) -> String {
        let now = Shown {
            step: installer.step.clone(),
            problem: installer.problem.clone(),
            checking: installer.checking,
        };
        let before = self.shown.replace(now.clone());
        if let Step::Making(phase) = &installer.step {
            let progress = words::progress(installer, *phase);
            return match before {
                Some(before) if before.step == now.step => format!("\r{progress}"),
                Some(Shown {
                    step: Step::Making(_),
                    ..
                }) => format!("\r\n{progress}"),
                _ => progress,
            };
        }
        let mut out = String::new();
        if matches!(
            before,
            Some(Shown {
                step: Step::Making(_),
                ..
            })
        ) {
            out.push_str("\r\n");
        }
        if before.as_ref() == Some(&now) {
            return out;
        }
        if now.checking {
            out.push_str("Checking it with its registry...\r\n");
            return out;
        }
        if let Some(problem) = &now.problem {
            let _ = write!(out, "Problem: {problem}\r\n");
        }
        out.push_str(&question(installer));
        out
    }

    fn resize(&mut self, _: u16, _: u16) {}

    fn close(&mut self) -> String {
        match self.shown {
            Some(Shown {
                step: Step::Making(_),
                ..
            }) => "\r\n".to_owned(),
            _ => String::new(),
        }
    }
}

fn numbered(out: &mut String, items: impl IntoIterator<Item = String>) {
    for (n, item) in items.into_iter().enumerate() {
        let _ = write!(out, "  {}) {item}\r\n", n + 1);
    }
}

fn question(installer: &Installer) -> String {
    let mut out = String::new();
    match &installer.step {
        Step::Name => {
            let current = match installer.name.as_str() {
                "" => String::new(),
                name => format!(" [{name}]"),
            };
            let _ = write!(
                out,
                "Name the sandbox (a-z, 0-9 and -, up to 32){current}: "
            );
        }
        Step::Image => {
            out.push_str("Images:\r\n");
            let mut items: Vec<String> = installer.offer.images.iter().map(words::image).collect();
            if installer.offer.named_images.is_some() {
                items.push(words::ANOTHER_IMAGE.to_owned());
            }
            numbered(&mut out, items);
            let _ = write!(out, "Image [{}]: ", installer.image + 1);
        }
        Step::Reference => {
            let _ = write!(out, "{}\r\n", words::named_images(installer));
            let current = match installer.reference.trim() {
                "" => String::new(),
                reference => format!(" [{reference}]"),
            };
            let _ = write!(out, "Image reference{current}: ");
        }
        Step::Size => {
            out.push_str("Sizes:\r\n");
            numbered(&mut out, installer.offer.sizes.iter().map(words::size));
            let _ = write!(out, "Size [{}]: ", installer.size + 1);
        }
        Step::Environment => {
            out.push_str("Environment entries to attach (their values stay hidden):\r\n");
            numbered(
                &mut out,
                installer
                    .offer
                    .entries
                    .iter()
                    .map(|entry| format!("{}  {}", entry.name, words::kind(entry))),
            );
            let chosen: Vec<String> = installer
                .chosen
                .iter()
                .enumerate()
                .filter(|&(_, &chosen)| chosen)
                .map(|(n, _)| (n + 1).to_string())
                .collect();
            let chosen = if chosen.is_empty() {
                "none".to_owned()
            } else {
                chosen.join(" ")
            };
            let _ = write!(out, "Entries, by number, or none [{chosen}]: ");
        }
        Step::Summary => {
            for line in super::screen::summary(installer, "Create it? [Y/n]: ") {
                out.push_str(&line.to_string());
                if !line.to_string().ends_with(": ") {
                    out.push_str("\r\n");
                }
            }
        }
        Step::Failed(reason) => {
            let _ = write!(
                out,
                "Could not create {}: {reason}\r\nNothing was made. Press Enter to answer \
                 again, or Esc to leave: ",
                installer.login()
            );
        }
        Step::Making(_) => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::Lines;
    use crate::lobby::View;
    use crate::lobby::installer::tests::offer;
    use crate::lobby::installer::{Effect, Event, Installer};

    /// Types `text` into the view, applies what it makes, and returns what the terminal shows.
    fn typed(lines: &mut Lines, installer: &mut Installer, text: &str) -> (String, Vec<Effect>) {
        let mut out = String::new();
        let mut effects = Vec::new();
        for event in lines.input(text.as_bytes(), &mut out) {
            effects.extend(installer.handle(event));
        }
        out.push_str(&lines.show(installer));
        (out, effects)
    }

    #[test]
    fn each_question_numbers_its_choices_and_an_answer_moves_on() {
        let mut installer = Installer::new(offer(Some("new")));
        let mut lines = Lines::new();
        let opened = lines.open(&installer);
        assert_eq!(
            opened,
            "Corral: there is no sandbox new.adam yet. Answer each question to make it, or \
             press Esc to leave.\r\nImages:\r\n  1) Base (base)\r\n  2) Tools (tools)\r\n  3) \
             Another image\r\nImage [2]: "
        );
        let (out, _) = typed(&mut lines, &mut installer, "9\r");
        assert_eq!(
            out,
            "9\r\nProblem: answer with a number from 1 to 3\r\nImages:\r\n  1) Base (base)\r\n  \
             2) Tools (tools)\r\n  3) Another image\r\nImage [2]: "
        );
        let (out, _) = typed(&mut lines, &mut installer, "\r");
        assert!(out.ends_with("Size [1]: "), "{out:?}");
        let (out, _) = typed(&mut lines, &mut installer, "2\r");
        assert!(
            out.ends_with(
                "  1) GH_TOKEN  variable, secret, for new sandboxes\r\n  2) .gitconfig  \
                 file\r\nEntries, by number, or none [1]: "
            ),
            "{out:?}"
        );
        let (out, _) = typed(&mut lines, &mut installer, "none\r");
        assert!(out.contains("Environment  none\r\n"), "{out:?}");
        assert!(out.ends_with("Create it? [Y/n]: "), "{out:?}");
        let (out, effects) = typed(&mut lines, &mut installer, "\r");
        assert_eq!(out, "\r\nCreating new.adam... 0 s");
        assert_eq!(effects.len(), 1);
        installer.handle(Event::Seconds(2));
        assert_eq!(lines.show(&installer), "\rCreating new.adam... 2 s");
        installer.handle(Event::Made(Ok(())));
        assert_eq!(lines.show(&installer), "\r\nStarting new.adam... 0 s");
        assert_eq!(lines.close(), "\r\n");
    }

    #[test]
    fn typing_echoes_and_esc_leaves() {
        let mut installer = Installer::new(offer(None));
        let mut lines = Lines::new();
        lines.open(&installer);
        let (out, effects) = typed(&mut lines, &mut installer, "nex\x7fw");
        assert_eq!((out.as_str(), effects.len()), ("nex\x08 \x08w", 0));
        let (_, effects) = typed(&mut lines, &mut installer, "\x1b");
        assert_eq!(effects, [Effect::Leave]);
    }

    #[test]
    fn a_refused_make_says_why_and_asks_again() {
        let mut installer = Installer::new(offer(Some("new")));
        let mut lines = Lines::new();
        lines.open(&installer);
        for _ in 0..4 {
            typed(&mut lines, &mut installer, "\r");
        }
        installer.handle(Event::Made(Err("at most 3 sandboxes per user".to_owned())));
        let shown = lines.show(&installer);
        assert_eq!(
            shown,
            "\r\nCould not create new.adam: at most 3 sandboxes per user\r\nNothing was made. \
             Press Enter to answer again, or Esc to leave: "
        );
        let (out, _) = typed(&mut lines, &mut installer, "\r");
        assert!(out.ends_with("Image [2]: "), "{out:?}");
    }
}
