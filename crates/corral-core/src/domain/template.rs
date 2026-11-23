use std::collections::BTreeSet;

use super::CredentialName;
use crate::{Error, Result};

const OPEN: &str = "{{";
const CLOSE: &str = "}}";

/// Text in which each `{{ NAME }}` stands for the value of the variable `NAME`, and nothing else
/// is special: no logic, no escapes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Template {
    parts: Vec<Part>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Part {
    Text(String),
    Variable(CredentialName),
}

impl Template {
    /// Refuses a `{{` without its `}}`, and one around anything but a variable name.
    pub fn parse(text: &str) -> Result<Self> {
        let mut parts = Vec::new();
        let mut rest = text;
        while let Some(open) = rest.find(OPEN) {
            let after = &rest[open + OPEN.len()..];
            let close = after
                .find(CLOSE)
                .ok_or_else(|| Error::Invalid("the template has a {{ without its }}".to_owned()))?;
            let inner = after[..close].trim();
            let name = CredentialName::parse(inner).map_err(|_| {
                Error::Invalid(format!(
                    "{{{{ {inner} }}}} in the template must name a variable, such as \
                     {{{{ GH_TOKEN }}}}"
                ))
            })?;
            if open > 0 {
                parts.push(Part::Text(rest[..open].to_owned()));
            }
            parts.push(Part::Variable(name));
            rest = &after[close + CLOSE.len()..];
        }
        if !rest.is_empty() {
            parts.push(Part::Text(rest.to_owned()));
        }
        Ok(Self { parts })
    }

    #[must_use]
    pub fn names(&self) -> BTreeSet<&CredentialName> {
        self.parts
            .iter()
            .filter_map(|part| match part {
                Part::Variable(name) => Some(name),
                Part::Text(_) => None,
            })
            .collect()
    }

    /// The text with each placeholder replaced by `value` of its name, or the names `value`
    /// does not know.
    pub fn render<'a>(
        &self,
        value: impl Fn(&CredentialName) -> Option<&'a str>,
    ) -> Result<String, BTreeSet<CredentialName>> {
        let mut rendered = String::new();
        let mut unknown = BTreeSet::new();
        for part in &self.parts {
            match part {
                Part::Text(text) => rendered.push_str(text),
                Part::Variable(name) => match value(name) {
                    Some(value) => rendered.push_str(value),
                    None => {
                        unknown.insert(name.clone());
                    }
                },
            }
        }
        if unknown.is_empty() {
            Ok(rendered)
        } else {
            Err(unknown)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::Template;
    use crate::Error;
    use crate::domain::CredentialName;

    fn name(value: &str) -> CredentialName {
        CredentialName::parse(value).unwrap()
    }

    #[test]
    fn placeholders_take_the_value_of_their_variable() {
        let template = Template::parse(
            "[user]\n\tname = {{GIT_NAME}}\n\ttoken = {{  GH_TOKEN }} {{GH_TOKEN}}",
        )
        .unwrap();
        assert_eq!(
            template.names(),
            BTreeSet::from([&name("GH_TOKEN"), &name("GIT_NAME")])
        );
        let rendered = template.render(|name| match name.as_str() {
            "GIT_NAME" => Some("Adam"),
            "GH_TOKEN" => Some("ghp_1"),
            _ => None,
        });
        assert_eq!(
            rendered.unwrap(),
            "[user]\n\tname = Adam\n\ttoken = ghp_1 ghp_1"
        );
    }

    #[test]
    fn text_without_placeholders_stays_as_it_is() {
        for text in ["", "plain", "a } and }} alone", "{ NAME }"] {
            let template = Template::parse(text).unwrap();
            assert!(template.names().is_empty(), "{text}");
            assert_eq!(template.render(|_| None).unwrap(), text);
        }
    }

    #[test]
    fn rendering_reports_every_name_it_does_not_know() {
        let template = Template::parse("{{ A }} {{ B }} {{ C }} {{ A }}").unwrap();
        let unknown = template
            .render(|name| (name.as_str() == "B").then_some("b"))
            .unwrap_err();
        assert_eq!(unknown, BTreeSet::from([name("A"), name("C")]));
    }

    #[test]
    fn anything_but_a_variable_name_between_braces_is_refused() {
        for (text, culprit) in [
            ("{{ lower }}", "{{ lower }}"),
            ("{{ .Values.x }}", "{{ .Values.x }}"),
            ("{{ A }}{{}}", "{{  }}"),
            ("{{ PATH }}", "{{ PATH }}"),
            ("open {{ A", "{{ without its }}"),
        ] {
            match Template::parse(text) {
                Err(Error::Invalid(message)) => assert!(message.contains(culprit), "{message}"),
                other => panic!("{text}: {other:?}"),
            }
        }
    }
}
