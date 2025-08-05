use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::net::TcpListener;

pub struct PortManager;

impl PortManager {
    pub fn check_port_availability(port: u16) -> bool {
        TcpListener::bind(("127.0.0.1", port)).is_ok()
    }

    pub fn check_ports_availability(ports: &[u16]) -> HashMap<u16, bool> {
        ports
            .iter()
            .map(|&port| (port, Self::check_port_availability(port)))
            .collect()
    }

    pub fn suggest_alternative_ports(
        used_ports: &HashSet<u16>,
        variable_ranges: &HashMap<String, (u16, u16)>,
    ) -> Result<HashMap<String, Vec<u16>>> {
        let mut suggestions = HashMap::new();

        for (variable, (start, end)) in variable_ranges {
            let mut available_ports = Vec::new();

            for port in *start..=*end {
                if !used_ports.contains(&port) && Self::check_port_availability(port) {
                    available_ports.push(port);
                    if available_ports.len() >= 5 {
                        // Limit suggestions
                        break;
                    }
                }
            }

            if !available_ports.is_empty() {
                suggestions.insert(variable.clone(), available_ports);
            }
        }

        Ok(suggestions)
    }

    pub fn get_system_reserved_ports() -> HashSet<u16> {
        // Common system reserved ports
        let mut reserved = HashSet::new();

        // Well-known ports (0-1023)
        for port in 1..=1023 {
            reserved.insert(port);
        }

        // Common development ports to avoid
        let common_ports = [3000, 3001, 8000, 8080, 8443, 8888, 9000, 9001];

        for &port in &common_ports {
            reserved.insert(port);
        }

        reserved
    }

    pub fn validate_port_ranges(ranges: &HashMap<String, (u16, u16)>) -> Result<Vec<String>> {
        let mut issues = Vec::new();
        let reserved_ports = Self::get_system_reserved_ports();

        for (variable, (start, end)) in ranges {
            if start >= end {
                issues.push(format!(
                    "Variable '{}': start port {} must be less than end port {}",
                    variable, start, end
                ));
                continue;
            }

            if *start == 0 {
                issues.push(format!("Variable '{}': port 0 is not valid", variable));
            }

            // Note: u16 can't exceed 65535, so no need to check upper bound

            // Check for overlap with reserved ports
            let range_overlaps_reserved = (*start..=*end).any(|p| reserved_ports.contains(&p));
            if range_overlaps_reserved {
                issues.push(format!(
                    "Variable '{}': port range {}-{} overlaps with system reserved ports",
                    variable, start, end
                ));
            }
        }

        // Check for overlapping ranges between variables
        let variables: Vec<_> = ranges.keys().collect();
        for i in 0..variables.len() {
            for j in (i + 1)..variables.len() {
                let (variable1, variable2) = (variables[i], variables[j]);
                let (start1, end1) = ranges[variable1];
                let (start2, end2) = ranges[variable2];

                if start1 <= end2 && end1 >= start2 {
                    issues.push(format!(
                        "Variables '{}' and '{}' have overlapping port ranges: {}-{} and {}-{}",
                        variable1, variable2, start1, end1, start2, end2
                    ));
                }
            }
        }

        Ok(issues)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_port_availability() {
        // Test with a port that should be available
        assert!(PortManager::check_port_availability(0)); // Port 0 lets OS choose

        // Test with a port that's likely to be unavailable (but this could be flaky)
        // We'll bind to a port first to make it unavailable
        let _listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = _listener.local_addr().unwrap();
        assert!(!PortManager::check_port_availability(addr.port()));
    }

    #[test]
    fn test_check_ports_availability() {
        let ports = vec![0, 65535]; // Port 0 should be available, 65535 might not be
        let availability = PortManager::check_ports_availability(&ports);

        assert_eq!(availability.len(), 2);
        assert!(availability.contains_key(&0));
        assert!(availability.contains_key(&65535));
    }

    #[test]
    fn test_get_system_reserved_ports() {
        let reserved = PortManager::get_system_reserved_ports();

        // Should contain well-known ports
        assert!(reserved.contains(&80)); // HTTP
        assert!(reserved.contains(&443)); // HTTPS
        assert!(reserved.contains(&22)); // SSH

        // Should contain common development ports
        assert!(reserved.contains(&3000));
        assert!(reserved.contains(&8080));
    }

    #[test]
    fn test_validate_port_ranges() -> Result<()> {
        let mut ranges = HashMap::new();
        ranges.insert("postgres".to_string(), (5432, 5500));
        ranges.insert("redis".to_string(), (6379, 6479));

        let issues = PortManager::validate_port_ranges(&ranges)?;
        assert!(issues.is_empty());

        // Test invalid range
        let mut invalid_ranges = HashMap::new();
        invalid_ranges.insert("invalid".to_string(), (5500, 5400)); // start > end

        let issues = PortManager::validate_port_ranges(&invalid_ranges)?;
        assert!(!issues.is_empty());
        assert!(issues[0].contains("start port"));

        Ok(())
    }

    #[test]
    fn test_validate_overlapping_ranges() -> Result<()> {
        let mut ranges = HashMap::new();
        ranges.insert("variable1".to_string(), (5000, 5100));
        ranges.insert("variable2".to_string(), (5050, 5150)); // Overlaps with variable1

        let issues = PortManager::validate_port_ranges(&ranges)?;
        assert!(!issues.is_empty());
        assert!(issues.iter().any(|issue| issue.contains("overlapping")));

        Ok(())
    }

    #[test]
    fn test_suggest_alternative_ports() -> Result<()> {
        let mut used_ports = HashSet::new();
        used_ports.insert(5432);
        used_ports.insert(5433);

        let mut variable_ranges = HashMap::new();
        variable_ranges.insert("postgres".to_string(), (5432, 5500));

        let suggestions = PortManager::suggest_alternative_ports(&used_ports, &variable_ranges)?;

        if let Some(postgres_suggestions) = suggestions.get("postgres") {
            assert!(!postgres_suggestions.is_empty());
            // Should not suggest already used ports
            assert!(!postgres_suggestions.contains(&5432));
            assert!(!postgres_suggestions.contains(&5433));
        }

        Ok(())
    }
}
