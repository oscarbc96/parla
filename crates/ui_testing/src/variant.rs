//! Which operating system and look a test renders in.

/// The operating system the tests run on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Macos,
    Windows,
    Linux,
}

impl Os {
    /// The OS this binary was built for.
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Os::Macos
        } else if cfg!(target_os = "windows") {
            Os::Windows
        } else {
            Os::Linux
        }
    }

    /// The scale factor machines of this OS usually run at.
    pub fn usual_scale(self) -> f32 {
        match self {
            Os::Macos => 2.0,
            Os::Windows | Os::Linux => 1.0,
        }
    }
}

/// Light or dark theme, chosen explicitly (the system appearance is ignored).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    Light,
    Dark,
}

/// One rendering variant of a scenario.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Variant {
    pub appearance: Appearance,
    pub scale: f32,
}

impl Variant {
    /// Light and dark at the OS's usual scale.
    pub fn defaults() -> Vec<Variant> {
        let scale = Os::current().usual_scale();
        [Appearance::Light, Appearance::Dark]
            .into_iter()
            .map(|appearance| Variant { appearance, scale })
            .collect()
    }
}
