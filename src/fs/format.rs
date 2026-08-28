/// Human-readable byte size, using binary (IEC) units.
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    if bytes == 0 {
        return "0 B".to_string();
    }
    let i = (bytes as f64).log(1024.0).floor() as usize;
    if i == 0 {
        return format!("{bytes} B");
    }
    let unit_index = i.min(UNITS.len() - 1);
    let value = bytes as f64 / 1024f64.powi(unit_index as i32);
    format!("{value:.2} {}", UNITS[unit_index])
}

/// First `num_lines` lines of `content`.
pub fn head_lines(content: &str, num_lines: u32) -> String {
    content
        .lines()
        .take(num_lines as usize)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Last `num_lines` lines of `content`.
pub fn tail_lines(content: &str, num_lines: u32) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(num_lines as usize);
    lines[start..].join("\n")
}

#[cfg(test)]
mod tests;
