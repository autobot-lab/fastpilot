use std::fmt;

use serde_yaml::Value as Yaml;

use crate::Point;

/// Semantic selector, resolved in this order (PRD §7.2). Never XPath.
#[derive(Debug, Clone, PartialEq)]
pub enum Selector {
    /// `e12` from the latest snapshot of the session.
    Ref(String),
    /// Visible text / accessibility label.
    Text(String),
    Role { role: String, name: Option<String> },
    /// resource-id / DOM id / accessibilityIdentifier.
    Id(String),
    Point(Point),
    /// The element that currently has input focus (`inputText: "..."` without `into`).
    Focused,
}

impl Selector {
    /// Parse a DSL selector. Strings shaped like `e12` are refs; other strings are text.
    pub(crate) fn from_yaml(v: &Yaml, extra_keys: &[&str]) -> Result<Self, String> {
        match v {
            Yaml::String(s) => Ok(Self::from_str_value(s)),
            Yaml::Number(n) => Ok(Self::Text(n.to_string())),
            Yaml::Mapping(map) => {
                let mut found = Vec::new();
                let mut name = None;
                for (k, val) in map {
                    let key = k.as_str().ok_or("selector keys must be strings")?;
                    match key {
                        "ref" | "text" | "id" | "role" | "point" => found.push((key, val)),
                        "name" => name = Some(scalar(val, "name")?),
                        k if extra_keys.contains(&k) => {}
                        other => {
                            return Err(format!(
                                "unknown selector key `{other}` (use ref, text, id, role+name or point)"
                            ));
                        }
                    }
                }
                let [(key, val)] = found.as_slice() else {
                    return Err(
                        "selector needs exactly one of: ref, text, id, role (+name), point".into(),
                    );
                };
                if name.is_some() && *key != "role" {
                    return Err("`name` is only valid together with `role`".into());
                }
                Ok(match *key {
                    "ref" => Self::Ref(scalar(val, "ref")?),
                    "text" => Self::Text(scalar(val, "text")?),
                    "id" => Self::Id(scalar(val, "id")?),
                    "role" => Self::Role {
                        role: scalar(val, "role")?,
                        name,
                    },
                    _ => Self::Point(point(val)?),
                })
            }
            _ => Err("selector must be a string or a map".into()),
        }
    }

    fn from_str_value(s: &str) -> Self {
        let is_ref = s.len() > 1
            && s.starts_with('e')
            && s[1..].bytes().all(|b| b.is_ascii_digit());
        if is_ref {
            Self::Ref(s.to_string())
        } else {
            Self::Text(s.to_string())
        }
    }
}

fn scalar(v: &Yaml, key: &str) -> Result<String, String> {
    match v {
        Yaml::String(s) => Ok(s.clone()),
        Yaml::Number(n) => Ok(n.to_string()),
        Yaml::Bool(b) => Ok(b.to_string()),
        _ => Err(format!("`{key}` must be a string")),
    }
}

fn point(v: &Yaml) -> Result<Point, String> {
    let err = || "`point` must be [x, y]".to_string();
    let seq = v.as_sequence().ok_or_else(err)?;
    match seq.as_slice() {
        [x, y] => Ok(Point {
            x: x.as_f64().ok_or_else(err)?,
            y: y.as_f64().ok_or_else(err)?,
        }),
        _ => Err(err()),
    }
}

impl fmt::Display for Selector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ref(r) => write!(f, "{r}"),
            Self::Text(t) => write!(f, "{t:?}"),
            Self::Role { role, name: Some(n) } => write!(f, "{{role: {role}, name: {n:?}}}"),
            Self::Role { role, name: None } => write!(f, "{{role: {role}}}"),
            Self::Id(id) => write!(f, "{{id: {id}}}"),
            Self::Point(p) => write!(f, "[{}, {}]", p.x, p.y),
            Self::Focused => f.write_str("(focused)"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sel(yaml: &str) -> Result<Selector, String> {
        let v: Yaml = serde_yaml::from_str(yaml).map_err(|e| e.to_string())?;
        Selector::from_yaml(&v, &[])
    }

    #[test]
    fn parses_all_forms() {
        assert_eq!(sel("Login"), Ok(Selector::Text("Login".into())));
        assert_eq!(sel("e3"), Ok(Selector::Ref("e3".into())));
        assert_eq!(sel("{ ref: e3 }"), Ok(Selector::Ref("e3".into())));
        assert_eq!(sel("{ id: btn_login }"), Ok(Selector::Id("btn_login".into())));
        assert_eq!(
            sel("{ role: button, name: Login }"),
            Ok(Selector::Role {
                role: "button".into(),
                name: Some("Login".into())
            })
        );
        assert_eq!(
            sel("{ point: [10, 20.5] }"),
            Ok(Selector::Point(Point { x: 10.0, y: 20.5 }))
        );
        assert!(sel("{ id: a, text: b }").is_err());
        assert!(sel("{ xpath: //a }").is_err());
    }
}
