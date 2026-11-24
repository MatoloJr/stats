//! Turning raw Windows performance counter instances into the figures
//! Task Manager shows.

/// GPU busy share the way Task Manager reports it: add up each engine's
/// utilisation across processes, then take the busiest engine.
pub fn gpu_busy(instances: &[(String, f64)]) -> Option<f64> {
    if instances.is_empty() {
        return None;
    }
    let mut engines: Vec<(&str, f64)> = Vec::new();
    for (name, value) in instances {
        // pid_1234_luid_0x0_0xD1A8_phys_0_eng_3_engtype_3D -> luid_..._engtype_3D
        let key = name.find("luid_").map(|i| &name[i..]).unwrap_or(name.as_str());
        match engines.iter_mut().find(|(k, _)| *k == key) {
            Some((_, total)) => *total += value,
            None => engines.push((key, *value)),
        }
    }
    engines.iter().map(|(_, v)| *v).reduce(f64::max).map(|v| v.clamp(0.0, 100.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_takes_busiest_engine_summed_over_processes() {
        let items = vec![
            ("pid_1_luid_0x0_0x1_phys_0_eng_0_engtype_3D".to_string(), 20.0),
            ("pid_2_luid_0x0_0x1_phys_0_eng_0_engtype_3D".to_string(), 15.0),
            ("pid_2_luid_0x0_0x1_phys_0_eng_3_engtype_VideoDecode".to_string(), 30.0),
        ];
        assert_eq!(gpu_busy(&items), Some(35.0));
        assert_eq!(gpu_busy(&[]), None);
    }
}
