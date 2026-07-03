use owo_colors::OwoColorize;

pub fn project_header(name: &str, lang: &str) {
    println!("\n{}", format!("━━━ {} ({}) ━━━", name, lang).bold());
}

pub fn success(msg: &str) {
    println!("{} {}", "✓".green().bold(), msg);
}

pub fn error(msg: &str) {
    eprintln!("{} {}", "✗".red().bold(), msg);
}

pub fn warn(msg: &str) {
    eprintln!("{} {}", "!".yellow().bold(), msg);
}

pub fn summary(succeeded: usize, total: usize, action: &str) {
    if total == 0 {
        return;
    }
    if succeeded == total {
        println!(
            "\n{}",
            format!("✓ {}/{} projects {} successfully", succeeded, total, action)
                .green()
                .bold()
        );
    } else {
        eprintln!(
            "\n{}",
            format!(
                "✗ {}/{} projects {} successfully ({} failed)",
                succeeded,
                total,
                action,
                total - succeeded
            )
            .red()
            .bold()
        );
    }
}
