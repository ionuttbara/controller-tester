use crate::config::registry;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RendererPreference {
    Auto,
    Gpu,
    Cpu,
}

impl RendererPreference {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Gpu => "gpu",
            Self::Cpu => "cpu",
        }
    }
}

#[derive(Clone, Debug)]
pub struct RendererSelection {
    pub status: String,
    pub preference: RendererPreference,
}

pub fn configure(args: &[String]) -> RendererSelection {
    // Command-line selection has highest priority. If no CLI override exists,
    // use the Registry preference. An explicitly supplied SLINT_BACKEND remains
    // respected when the stored preference is Auto.
    let stored = registry::load_settings().renderer;
    let requested = parse_renderer(args).or_else(|| parse_value(&stored));
    let preference = requested.unwrap_or(RendererPreference::Auto);

    match preference {
        RendererPreference::Gpu => {
            std::env::set_var("SLINT_BACKEND", "winit-femtovg");
            RendererSelection {
                status: "GPU 2D • FemtoVG".to_string(),
                preference,
            }
        }
        RendererPreference::Cpu => {
            std::env::set_var("SLINT_BACKEND", "winit-software");
            RendererSelection {
                status: "CPU 2D • Slint software".to_string(),
                preference,
            }
        }
        RendererPreference::Auto => {
            if parse_renderer(args).is_some() {
                std::env::remove_var("SLINT_BACKEND");
            }
            let status = match std::env::var("SLINT_BACKEND") {
                Ok(existing) if !existing.trim().is_empty() => format!("2D renderer • {existing}"),
                _ => "AUTO 2D • GPU → CPU fallback".to_string(),
            };
            RendererSelection { status, preference }
        }
    }
}

fn parse_renderer(args: &[String]) -> Option<RendererPreference> {
    let mut iter = args.iter().skip(1).peekable();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--gpu-renderer" => return Some(RendererPreference::Gpu),
            "--cpu-renderer" => return Some(RendererPreference::Cpu),
            "--renderer" => {
                if let Some(value) = iter.next() {
                    return parse_value(value);
                }
            }
            _ => {
                if let Some(value) = arg.strip_prefix("--renderer=") {
                    return parse_value(value);
                }
            }
        }
    }
    None
}

fn parse_value(value: &str) -> Option<RendererPreference> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(RendererPreference::Auto),
        "gpu" | "femtovg" => Some(RendererPreference::Gpu),
        "cpu" | "software" => Some(RendererPreference::Cpu),
        _ => None,
    }
}
