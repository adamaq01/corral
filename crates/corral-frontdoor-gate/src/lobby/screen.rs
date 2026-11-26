//! The lobby on a full screen: the alternate screen of the client's terminal, drawn with ratatui
//! and driven with keys.

use ratatui::layout::{Constraint, Layout, Rect, Size};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Padding, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use super::View;
use super::backend::ChannelBackend;
use super::installer::{Event, Field, Installer, Step};
use super::keys::keys;
use super::words;

const ENTER: &str = "\x1b[?1049h";
const LEAVE: &str = "\x1b[0m\x1b[?25h\x1b[?1049l";

pub(crate) struct Screen {
    terminal: Terminal<ChannelBackend>,
}

impl Screen {
    pub(crate) fn new(cols: u16, rows: u16, color: bool) -> Self {
        let backend = ChannelBackend::new(Size::new(cols, rows), color);
        Self {
            terminal: Terminal::new(backend).expect("a channel backend never fails"),
        }
    }

    fn draw(&mut self, installer: &Installer) -> String {
        let _ = self.terminal.draw(|frame| render(installer, frame));
        self.terminal.backend_mut().take()
    }
}

impl View for Screen {
    fn open(&mut self, installer: &Installer) -> String {
        self.terminal.backend_mut().write(ENTER);
        let _ = self.terminal.clear();
        self.draw(installer)
    }

    fn input(&mut self, data: &[u8], _: &mut String) -> Vec<Event> {
        keys(data).into_iter().map(Event::Key).collect()
    }

    fn show(&mut self, installer: &Installer) -> String {
        self.draw(installer)
    }

    fn resize(&mut self, cols: u16, rows: u16) {
        self.terminal
            .backend_mut()
            .resize(Size::new(cols.max(1), rows.max(1)));
    }

    fn close(&mut self) -> String {
        let mut out = self.terminal.backend_mut().take();
        out.push_str(LEAVE);
        out
    }
}

fn picked() -> Style {
    Style::new()
        .add_modifier(Modifier::REVERSED | Modifier::BOLD)
        .fg(Color::Cyan)
}

fn problem(text: &str) -> Line<'_> {
    Line::from(Span::styled(
        format!("Problem: {text}"),
        Style::new().add_modifier(Modifier::BOLD).fg(Color::Red),
    ))
}

pub(crate) fn render(installer: &Installer, frame: &mut Frame) {
    let title = if installer.name.is_empty() || installer.step == Step::Name {
        format!(" Corral: a new sandbox for {} ", installer.offer.handle)
    } else {
        format!(" Corral: new sandbox {} ", installer.login())
    };
    let block = Block::bordered()
        .padding(Padding::horizontal(1))
        .title(Span::styled(
            title,
            Style::new().add_modifier(Modifier::BOLD).fg(Color::Cyan),
        ));
    let inner = block.inner(frame.area());
    frame.render_widget(block, frame.area());
    let [guide, _, body, hints] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(inner);
    frame.render_widget(guide_line(installer), guide);
    frame.render_widget(
        Line::from(Span::styled(
            hint(&installer.step),
            Style::new().add_modifier(Modifier::DIM),
        )),
        hints,
    );
    body_of(installer, frame, body);
}

fn body_of(installer: &Installer, frame: &mut Frame, body: Rect) {
    match &installer.step {
        Step::Name => text_field(
            installer,
            frame,
            body,
            vec![
                Line::from(format!(
                    "{} has no sandbox yet. Name the first one (a-z, 0-9 and -, up to 32):",
                    installer.offer.handle
                )),
                Line::from(""),
            ],
            &installer.name,
        ),
        Step::Reference => {
            let mut above = vec![
                Line::from("Name the image to make the sandbox from:"),
                Line::from(words::named_images(installer)),
                Line::from(""),
            ];
            if installer.checking {
                above.push(Line::from("Checking it with its registry..."));
            }
            text_field(installer, frame, body, above, &installer.reference);
        }
        Step::Image => {
            let mut items: Vec<String> = installer
                .offer
                .images
                .iter()
                .map(|image| {
                    defaulted(
                        words::image(image),
                        image.key == installer.offer.default_image,
                    )
                })
                .collect();
            if installer.offer.named_images.is_some() {
                items.push(words::ANOTHER_IMAGE.to_owned());
            }
            list(frame, body, "Choose the image:", items, installer.image);
        }
        Step::Size => {
            let items = installer
                .offer
                .sizes
                .iter()
                .map(|size| defaulted(words::size(size), size.key == installer.offer.default_size))
                .collect();
            list(frame, body, "Choose the size:", items, installer.size);
        }
        Step::Setup => {
            let items = installer
                .offer
                .entries
                .iter()
                .zip(&installer.chosen)
                .map(|(entry, &chosen)| {
                    if entry.required() {
                        return words::fixed(entry);
                    }
                    let mark = if chosen { "[x]" } else { "[ ]" };
                    format!("{mark} {}  {}", entry.name, words::kind(entry))
                })
                .collect();
            list(
                frame,
                body,
                "Choose the setup entries to attach (their values stay hidden):",
                items,
                installer.entry,
            );
        }
        Step::Repository(field) => repository(installer, frame, body, *field),
        Step::Summary => paragraph(frame, body, summary(installer, "Create it?")),
        Step::Making(phase) => paragraph(
            frame,
            body,
            vec![
                Line::from(words::progress(installer, *phase)),
                Line::from(""),
                Line::from("Its shell opens here once it is up."),
            ],
        ),
        Step::Failed(reason) => paragraph(
            frame,
            body,
            vec![
                Line::from(format!("Could not create {}:", installer.login())),
                Line::from(Span::styled(
                    reason.as_str(),
                    Style::new().add_modifier(Modifier::BOLD).fg(Color::Red),
                )),
                Line::from(""),
                Line::from("Nothing was made. Go back to change your choices."),
            ],
        ),
    }
}

/// The repository's URL, or its branch under the URL given.
fn repository(installer: &Installer, frame: &mut Frame, body: Rect, field: Field) {
    let mut above = vec![Line::from(
        "Clone a repository into the new sandbox? Leave empty for none.",
    )];
    match field {
        Field::Url => {
            above.push(Line::from(""));
            text_field(installer, frame, body, above, &installer.url);
        }
        Field::Branch => {
            above.extend([
                Line::from(format!("  {}", installer.url.trim())),
                Line::from(""),
                Line::from("Branch (empty for the default):"),
                Line::from(""),
            ]);
            text_field(installer, frame, body, above, &installer.branch);
        }
    }
}

fn defaulted(text: String, default: bool) -> String {
    if default {
        format!("{text}  (your default)")
    } else {
        text
    }
}

fn guide_line(installer: &Installer) -> Line<'static> {
    let current = words::step(&installer.step);
    let mut spans = Vec::new();
    let mut seen = Vec::new();
    for step in installer.steps() {
        let name = words::step(&step);
        if seen.contains(&name) {
            continue;
        }
        if !seen.is_empty() {
            spans.push(Span::raw(" > "));
        }
        seen.push(name);
        spans.push(if name == current {
            Span::styled(
                name,
                Style::new().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            )
        } else {
            Span::styled(name, Style::new().add_modifier(Modifier::DIM))
        });
    }
    Line::from(spans)
}

fn hint(step: &Step) -> &'static str {
    match step {
        Step::Name => "Enter: next  Esc: leave",
        Step::Reference => "Enter: check it and go on  Up: back  Esc: leave",
        Step::Repository(_) => "Enter: next  Up: back  Esc: leave",
        Step::Image | Step::Size => "Up/Down: choose  Enter: next  Left: back  Esc or q: leave",
        Step::Setup => "Up/Down: move  Space: check  Enter: next  Left: back  Esc or q: leave",
        Step::Summary => "Enter: create  Left: back  Esc or q: leave",
        Step::Making(_) => "",
        Step::Failed(_) => "Enter: back to the summary  Esc or q: leave",
    }
}

pub(crate) fn summary(installer: &Installer, question: &'static str) -> Vec<Line<'static>> {
    vec![
        Line::from(format!("Sandbox      {}", installer.login())),
        Line::from(format!("Image        {}", words::chosen_image(installer))),
        Line::from(format!("Size         {}", words::chosen_size(installer))),
        Line::from(format!("Setup        {}", words::chosen_setup(installer))),
        Line::from(format!(
            "Repository   {}",
            words::chosen_repository(installer)
        )),
        Line::from(""),
        Line::from(question),
    ]
}

fn paragraph(frame: &mut Frame, area: Rect, lines: Vec<Line<'_>>) {
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn list(frame: &mut Frame, area: Rect, heading: &str, items: Vec<String>, selected: usize) {
    let [heading_area, _, list_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .areas(area);
    frame.render_widget(Line::from(heading), heading_area);
    let list = List::new(items.into_iter().map(ListItem::new))
        .highlight_symbol("> ")
        .highlight_style(picked());
    let mut state = ListState::default().with_selected(Some(selected));
    frame.render_stateful_widget(list, list_area, &mut state);
}

/// A field of text below `above`, with the cursor at its end and the step's problem under it.
fn text_field<'a>(
    installer: &'a Installer,
    frame: &mut Frame,
    area: Rect,
    mut above: Vec<Line<'a>>,
    text: &str,
) {
    let row = u16::try_from(above.len()).unwrap_or(u16::MAX);
    above.push(Line::from(format!("> {text}")));
    if let Some(text) = &installer.problem {
        above.push(Line::from(""));
        above.push(problem(text));
    }
    frame.render_widget(Paragraph::new(above).wrap(Wrap { trim: false }), area);
    let column = u16::try_from(text.chars().count() + 2).unwrap_or(u16::MAX);
    if !installer.checking && column < area.width && row < area.height {
        frame.set_cursor_position((area.x + column, area.y + row));
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::render;
    use crate::lobby::installer::tests::offer;
    use crate::lobby::installer::{Event, Field, Installer, Phase, Step};
    use crate::lobby::keys::Key;

    /// The screen at 80x24, line by line.
    pub(crate) fn drawn(installer: &Installer) -> String {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| render(installer, frame)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..24)
            .map(|y| {
                (0..80)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn at(step: Step) -> Installer {
        let mut installer = Installer::new(offer(Some("new")));
        installer.step = step;
        installer
    }

    #[test]
    fn each_step_fits_80_by_24_with_its_choices_and_its_keys() {
        let screen = drawn(&at(Step::Image));
        for wanted in [
            " Corral: new sandbox new.adam ",
            "Image > Size > Setup > Repository > Summary",
            "Choose the image:",
            "  Base (base)",
            "> Tools (tools)  (your default)",
            "  Another image",
            "Up/Down: choose  Enter: next  Left: back  Esc or q: leave",
        ] {
            assert!(screen.contains(wanted), "no {wanted:?} in\n{screen}");
        }
        let screen = drawn(&at(Step::Size));
        assert!(
            screen.contains("> Small (small): 1 CPU, 2 GiB memory, 10 GiB home  (your default)"),
            "{screen}"
        );
        let screen = drawn(&at(Step::Setup));
        for wanted in [
            "  = ANTHROPIC_BASE_URL  variable, required",
            "> [x] GH_TOKEN  variable, secret, for new sandboxes",
            "  [ ] .gitconfig  file",
            "Space: check",
        ] {
            assert!(screen.contains(wanted), "no {wanted:?} in\n{screen}");
        }
        let screen = drawn(&at(Step::Repository(Field::Url)));
        for wanted in [
            "Clone a repository into the new sandbox? Leave empty for none.",
            "Enter: next  Up: back  Esc: leave",
        ] {
            assert!(screen.contains(wanted), "no {wanted:?} in\n{screen}");
        }
        let mut branch = at(Step::Repository(Field::Branch));
        branch.url = "https://github.com/adamaq01/corral.git".to_owned();
        branch.branch = "main".to_owned();
        let screen = drawn(&branch);
        for wanted in [
            "https://github.com/adamaq01/corral.git",
            "Branch (empty for the default):",
            "> main",
        ] {
            assert!(screen.contains(wanted), "no {wanted:?} in\n{screen}");
        }
        branch.step = Step::Summary;
        let screen = drawn(&branch);
        for wanted in [
            "Sandbox      new.adam",
            "Image        Tools (tools)",
            "Setup        ANTHROPIC_BASE_URL, GH_TOKEN",
            "Repository   https://github.com/adamaq01/corral.git (main)",
            "Enter: create",
        ] {
            assert!(screen.contains(wanted), "no {wanted:?} in\n{screen}");
        }
        assert!(
            drawn(&at(Step::Summary)).contains("Repository   none"),
            "{screen}"
        );
        let mut making = at(Step::Making(Phase::Starting));
        making.handle(Event::Seconds(12));
        assert!(drawn(&making).contains("Starting new.adam... 12 s"));
        let screen = drawn(&at(Step::Failed("at most 3 sandboxes per user".to_owned())));
        assert!(screen.contains("Could not create new.adam:"), "{screen}");
        assert!(screen.contains("at most 3 sandboxes per user"), "{screen}");
    }

    #[test]
    fn a_text_field_shows_what_is_typed_and_its_problem() {
        let mut installer = Installer::new(offer(None));
        for c in "Bad".chars() {
            installer.handle(Event::Key(Key::Char(c)));
        }
        installer.handle(Event::Key(Key::Enter));
        let screen = drawn(&installer);
        for wanted in [
            " Corral: a new sandbox for adam ",
            "Name > Image > Size > Setup > Repository > Summary",
            "adam has no sandbox yet.",
            "> Bad",
            "Problem: sandbox name \"Bad\" must match [a-z0-9-]{1,32}",
        ] {
            assert!(screen.contains(wanted), "no {wanted:?} in\n{screen}");
        }
        let mut installer = at(Step::Reference);
        installer.image = 2;
        installer.checking = true;
        let screen = drawn(&installer);
        assert!(
            screen.contains("An image reference from ghcr.io/acme, at most 500 MiB to download."),
            "{screen}"
        );
        assert!(
            screen.contains("Checking it with its registry..."),
            "{screen}"
        );
        assert!(screen.contains("Image > Size"), "{screen}");
    }

    #[test]
    #[ignore = "prints each screen, for a person to look at"]
    fn print_every_screen() {
        let mut bare = Installer::new(offer(None));
        bare.name = "new".to_owned();
        let mut reference = at(Step::Reference);
        reference.image = 2;
        reference.reference = "debian:13".to_owned();
        reference.problem =
            Some("debian:13 is from none of the registries sandboxes may use: ghcr.io/acme".into());
        let mut url = at(Step::Repository(Field::Url));
        url.url = "https://me:tok3n@github.com/me/x.git".to_owned();
        url.problem = Some(
            "the repository URL carries a password; put the token in Setup, as a git \
             credential, and leave it out of the URL"
                .into(),
        );
        let mut branch = at(Step::Repository(Field::Branch));
        branch.url = "https://github.com/adamaq01/corral.git".to_owned();
        branch.branch = "-x".to_owned();
        branch.problem = Some("branch \"-x\" is not a git branch name".into());
        let mut summary = at(Step::Summary);
        summary.url = "https://github.com/adamaq01/corral.git".to_owned();
        summary.branch = "main".to_owned();
        let mut making = at(Step::Making(Phase::Creating));
        making.seconds = 3;
        for installer in [
            bare,
            at(Step::Image),
            reference,
            at(Step::Size),
            at(Step::Setup),
            url,
            branch,
            summary,
            making,
            at(Step::Failed(
                "template ~/.config/tool.toml refers to EXTRA: choose it too, or leave the \
                 template out"
                    .to_owned(),
            )),
        ] {
            println!("{}\n", drawn(&installer));
        }
    }
}
