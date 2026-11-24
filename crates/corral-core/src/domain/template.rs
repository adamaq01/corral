use std::collections::BTreeSet;

use super::VariableName;

const OPEN: &str = "{{";
const CLOSE: &str = "}}";

/// Text in which each `{{ NAME }}`, `NAME` a variable name in upper case, stands for the value of
/// that variable. Any other `{{` is text, so the syntax of Go, Jinja or Helm passes through.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Template {
    parts: Vec<Part>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Part {
    Text(String),
    Variable(VariableName),
}

impl Template {
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let mut parts = Vec::new();
        let mut literal = String::new();
        let mut rest = text;
        while let Some(open) = rest.find(OPEN) {
            let after = &rest[open + OPEN.len()..];
            if let Some((name, length)) = reference(after) {
                literal.push_str(&rest[..open]);
                if !literal.is_empty() {
                    parts.push(Part::Text(std::mem::take(&mut literal)));
                }
                parts.push(Part::Variable(name));
                rest = &after[length..];
            } else {
                // One brace at a time, so `{{{ A }}}` still finds `{{ A }}`.
                literal.push_str(&rest[..=open]);
                rest = &rest[open + 1..];
            }
        }
        literal.push_str(rest);
        if !literal.is_empty() {
            parts.push(Part::Text(literal));
        }
        Self { parts }
    }

    #[must_use]
    pub fn names(&self) -> BTreeSet<&VariableName> {
        self.parts
            .iter()
            .filter_map(|part| match part {
                Part::Variable(name) => Some(name),
                Part::Text(_) => None,
            })
            .collect()
    }

    /// The text with each reference replaced by `value` of its name, or the names `value` does
    /// not know.
    pub fn render<'a>(
        &self,
        value: impl Fn(&VariableName) -> Option<&'a str>,
    ) -> Result<String, BTreeSet<VariableName>> {
        let mut unknown = BTreeSet::new();
        let rendered = self.render_with(|name| {
            value(name).map_or_else(
                || {
                    unknown.insert(name.clone());
                    String::new()
                },
                str::to_owned,
            )
        });
        if unknown.is_empty() {
            Ok(rendered)
        } else {
            Err(unknown)
        }
    }

    /// The text with each reference replaced by what `value` gives for its name.
    pub fn render_with(&self, mut value: impl FnMut(&VariableName) -> String) -> String {
        let mut rendered = String::new();
        for part in &self.parts {
            match part {
                Part::Text(text) => rendered.push_str(text),
                Part::Variable(name) => rendered.push_str(&value(name)),
            }
        }
        rendered
    }
}

/// The name and the length of a reference that `after`, the text after a `{{`, starts with.
fn reference(after: &str) -> Option<(VariableName, usize)> {
    let close = after.find(CLOSE)?;
    let name = VariableName::parse(after[..close].trim())
        .ok()
        .filter(VariableName::is_referable)?;
    Some((name, close + CLOSE.len()))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::Template;
    use crate::domain::VariableName;

    fn name(value: &str) -> VariableName {
        VariableName::parse(value).unwrap()
    }

    #[test]
    fn references_take_the_value_of_their_variable() {
        let template = Template::parse(
            "[user]\n\tname = {{GIT_NAME}}\n\ttoken = {{  GH_TOKEN }} {{GH_TOKEN}}",
        );
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
    fn anything_but_a_reference_stays_as_it_is() {
        for text in [
            "",
            "plain",
            "a } and }} alone",
            "{ NAME }",
            "{{ lower }} {{ end }} {{- end }}",
            "{{ .Values.image.tag | quote }}",
            "{% if x %}{{ user.name }}{% endif %}",
            "{{ PATH }} {{ CORRAL_X }}",
            "{{ A B }} {{}} {{ 1A }}",
            "open {{ A",
            "{{\"{{\"}}",
        ] {
            let template = Template::parse(text);
            assert!(template.names().is_empty(), "{text}");
            assert_eq!(template.render(|_| None).unwrap(), text);
        }
    }

    #[test]
    fn references_sit_among_other_braces() {
        let template = Template::parse("{{{ A }}} {{ .x }}{{ A }} {{ B");
        assert_eq!(template.names(), BTreeSet::from([&name("A")]));
        assert_eq!(
            template.render(|_| Some("a")).unwrap(),
            "{a} {{ .x }}a {{ B"
        );
    }

    #[test]
    fn rendering_reports_every_name_it_does_not_know() {
        let template = Template::parse("{{ A }} {{ B }} {{ C }} {{ A }}");
        let unknown = template
            .render(|name| (name.as_str() == "B").then_some("b"))
            .unwrap_err();
        assert_eq!(unknown, BTreeSet::from([name("A"), name("C")]));
        assert_eq!(
            template.render_with(|name| format!("<{name}>")),
            "<A> <B> <C> <A>"
        );
    }
}
