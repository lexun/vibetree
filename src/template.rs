use anyhow::{Context, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashMap;

static COMPONENT_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\{(port|int):(\d+)\}").expect("Failed to compile component regex")
});

/// Component types that can appear in templates
#[derive(Debug, Clone, PartialEq)]
pub enum ComponentType {
    /// Port allocation - checks availability and allocates next available port
    Port(u16),
    /// Integer increment - simple counter-based allocation
    Int(u16),
}

/// A parsed component from a template string
#[derive(Debug, Clone, PartialEq)]
pub struct TemplateComponent {
    pub component_type: ComponentType,
    pub start_pos: usize,
    pub end_pos: usize,
}

/// Result of parsing a template string
#[derive(Debug, Clone)]
pub struct ParsedTemplate {
    pub original: String,
    pub components: Vec<TemplateComponent>,
}

impl ParsedTemplate {
    /// Parse a template string and extract all components
    pub fn parse(template: &str) -> Result<Self> {
        let mut components = Vec::new();

        for cap in COMPONENT_REGEX.captures_iter(template) {
            let full_match = cap.get(0).unwrap();
            let component_type_str = cap.get(1).unwrap().as_str();
            let value_str = cap.get(2).unwrap().as_str();

            let value: u16 = value_str
                .parse()
                .with_context(|| format!("Invalid value in component: {}", value_str))?;

            let component_type = match component_type_str {
                "port" => ComponentType::Port(value),
                "int" => ComponentType::Int(value),
                _ => anyhow::bail!("Unknown component type: {}", component_type_str),
            };

            components.push(TemplateComponent {
                component_type,
                start_pos: full_match.start(),
                end_pos: full_match.end(),
            });
        }

        Ok(ParsedTemplate {
            original: template.to_string(),
            components,
        })
    }

    /// Check if this template has any components
    pub fn has_components(&self) -> bool {
        !self.components.is_empty()
    }

    /// Resolve the template by replacing components with allocated values
    pub fn resolve(&self, allocated_values: &HashMap<usize, String>) -> Result<String> {
        if self.components.is_empty() {
            return Ok(self.original.clone());
        }

        let mut result = String::new();
        let mut last_pos = 0;

        for (idx, component) in self.components.iter().enumerate() {
            // Add the text before this component
            result.push_str(&self.original[last_pos..component.start_pos]);

            // Add the allocated value for this component
            let value = allocated_values
                .get(&idx)
                .with_context(|| format!("No allocated value for component {}", idx))?;
            result.push_str(value);

            last_pos = component.end_pos;
        }

        // Add any remaining text after the last component
        result.push_str(&self.original[last_pos..]);

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_empty_template() -> Result<()> {
        let parsed = ParsedTemplate::parse("simple_string")?;
        assert_eq!(parsed.original, "simple_string");
        assert_eq!(parsed.components.len(), 0);
        assert!(!parsed.has_components());
        Ok(())
    }

    #[test]
    fn test_parse_port_component() -> Result<()> {
        let parsed = ParsedTemplate::parse("{port:3000}")?;
        assert_eq!(parsed.components.len(), 1);
        assert_eq!(
            parsed.components[0].component_type,
            ComponentType::Port(3000)
        );
        Ok(())
    }

    #[test]
    fn test_parse_int_component() -> Result<()> {
        let parsed = ParsedTemplate::parse("{int:1}")?;
        assert_eq!(parsed.components.len(), 1);
        assert_eq!(parsed.components[0].component_type, ComponentType::Int(1));
        Ok(())
    }

    #[test]
    fn test_parse_multiple_components() -> Result<()> {
        let parsed = ParsedTemplate::parse("server_{port:3000}_v{int:2}")?;
        assert_eq!(parsed.components.len(), 2);
        assert_eq!(
            parsed.components[0].component_type,
            ComponentType::Port(3000)
        );
        assert_eq!(parsed.components[1].component_type, ComponentType::Int(2));
        Ok(())
    }

    #[test]
    fn test_parse_multiple_same_type() -> Result<()> {
        let parsed = ParsedTemplate::parse("server_{port:3000}_admin_{port:3001}")?;
        assert_eq!(parsed.components.len(), 2);
        assert_eq!(
            parsed.components[0].component_type,
            ComponentType::Port(3000)
        );
        assert_eq!(
            parsed.components[1].component_type,
            ComponentType::Port(3001)
        );
        Ok(())
    }

    #[test]
    fn test_resolve_simple() -> Result<()> {
        let parsed = ParsedTemplate::parse("simple")?;
        let resolved = parsed.resolve(&HashMap::new())?;
        assert_eq!(resolved, "simple");
        Ok(())
    }

    #[test]
    fn test_resolve_with_components() -> Result<()> {
        let parsed = ParsedTemplate::parse("server_{port:3000}_v{int:2}")?;
        let mut allocated = HashMap::new();
        allocated.insert(0, "3001".to_string());
        allocated.insert(1, "3".to_string());

        let resolved = parsed.resolve(&allocated)?;
        assert_eq!(resolved, "server_3001_v3");
        Ok(())
    }

    #[test]
    fn test_resolve_complex_template() -> Result<()> {
        let parsed = ParsedTemplate::parse("postgres_{port:5432}_data_{int:1}_suffix")?;
        let mut allocated = HashMap::new();
        allocated.insert(0, "5433".to_string());
        allocated.insert(1, "2".to_string());

        let resolved = parsed.resolve(&allocated)?;
        assert_eq!(resolved, "postgres_5433_data_2_suffix");
        Ok(())
    }

    #[test]
    fn test_invalid_component_type() {
        let result = ParsedTemplate::parse("{invalid:123}");
        assert!(result.is_ok()); // It should parse but find no components
        let parsed = result.unwrap();
        assert_eq!(parsed.components.len(), 0); // Invalid types are ignored
    }

    #[test]
    fn test_edge_case_zero_value() -> Result<()> {
        let parsed = ParsedTemplate::parse("prefix_{int:0}_suffix")?;
        assert_eq!(parsed.components.len(), 1);
        assert_eq!(parsed.components[0].component_type, ComponentType::Int(0));
        Ok(())
    }
}
