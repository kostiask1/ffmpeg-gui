pub fn parse_time(line: &str) -> Option<f64> {
    if let Some(value) = line.strip_prefix("out_time_us=") {
        return value.trim().parse::<f64>().ok().map(|v| v / 1_000_000.0);
    }
    if let Some(value) = line.strip_prefix("out_time_ms=") {
        // FFmpeg historically labels this field ms although its unit is microseconds.
        return value.trim().parse::<f64>().ok().map(|v| v / 1_000_000.0);
    }
    None
}

pub fn parse_speed(line: &str) -> Option<f64> {
    line.strip_prefix("speed=")?
        .trim()
        .trim_end_matches('x')
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_machine_progress() {
        assert_eq!(parse_time("out_time_us=12500000"), Some(12.5));
        assert_eq!(parse_speed("speed=2.40x"), Some(2.4));
    }
}
