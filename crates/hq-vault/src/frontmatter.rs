use anyhow::Result;
use std::collections::HashMap;

/// Parse a markdown file into frontmatter + content body.
pub(crate) fn parse(raw: &str) -> Result<(HashMap<String, serde_yaml::Value>, String)> {
    let matter = gray_matter::Matter::<gray_matter::engine::YAML>::new();
    let result = matter.parse(raw);

    let frontmatter: HashMap<String, serde_yaml::Value> = match result.data {
        Some(gray_matter::Pod::Hash(map)) => {
            let mut fm = HashMap::new();
            for (k, v) in map {
                fm.insert(k, pod_to_yaml_value(&v));
            }
            fm
        }
        _ => HashMap::new(),
    };

    Ok((frontmatter, result.content))
}

/// Serialize frontmatter + content back to a markdown string.
pub(crate) fn serialize(
    frontmatter: &HashMap<String, serde_yaml::Value>,
    content: &str,
) -> Result<String> {
    if frontmatter.is_empty() {
        return Ok(content.to_string());
    }

    let yaml = serde_yaml::to_string(frontmatter)?;
    Ok(format!("---\n{}---\n\n{}", yaml, content))
}

pub fn pod_to_yaml_value(pod: &gray_matter::Pod) -> serde_yaml::Value {
    match pod {
        gray_matter::Pod::String(s) => serde_yaml::Value::String(s.clone()),
        gray_matter::Pod::Integer(i) => serde_yaml::Value::Number(serde_yaml::Number::from(*i)),
        gray_matter::Pod::Float(f) => serde_yaml::Value::Number(serde_yaml::Number::from(*f)),
        gray_matter::Pod::Boolean(b) => serde_yaml::Value::Bool(*b),
        gray_matter::Pod::Null => serde_yaml::Value::Null,
        gray_matter::Pod::Array(arr) => {
            serde_yaml::Value::Sequence(arr.iter().map(pod_to_yaml_value).collect())
        }
        gray_matter::Pod::Hash(map) => {
            let mapping: serde_yaml::Mapping = map
                .iter()
                .map(|(k, v)| (serde_yaml::Value::String(k.clone()), pod_to_yaml_value(v)))
                .collect();
            serde_yaml::Value::Mapping(mapping)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_frontmatter_and_content() {
        let raw =
            "---\ntitle: Hello World\ntags:\n  - rust\n  - hq\n---\n\n# Hello\n\nThis is content.";
        let (fm, content) = parse(raw).unwrap();
        assert_eq!(
            fm.get("title").unwrap(),
            &serde_yaml::Value::String("Hello World".to_string())
        );
        assert!(content.contains("# Hello"));
    }

    #[test]
    fn parse_no_frontmatter() {
        let raw = "# Just markdown\n\nNo frontmatter here.";
        let (fm, content) = parse(raw).unwrap();
        assert!(fm.is_empty());
        assert!(content.contains("# Just markdown"));
    }

    #[test]
    fn roundtrip() {
        let mut fm = HashMap::new();
        fm.insert(
            "title".to_string(),
            serde_yaml::Value::String("Test".to_string()),
        );
        let content = "# Test\n\nBody text.";
        let serialized = serialize(&fm, content).unwrap();
        let (fm2, content2) = parse(&serialized).unwrap();
        assert_eq!(fm2.get("title"), fm.get("title"));
        assert!(content2.contains("Body text."));
    }
}
