/// Backend-independent settings retained by a running compositor session.
///
/// Configuration parsing remains in `swm-config`; this aggregate owns the
/// applied values and their reload lifecycle. Keeping them together prevents
/// the session coordinator from exposing each setting as unrelated mutable
/// state.
pub struct RuntimeSettings {
    pub overlays: swm_config::Overlays,
    configured_overlays: swm_config::Overlays,
    system_color_scheme: swm_config::SystemColorScheme,
    pub input: swm_config::Input,
    pub animations: swm_config::Animations,
    pub decorations: swm_config::Decorations,
    pub font: swm_config::Font,
    pub background: swm_config::Background,
    pub field: swm_config::Field,
    pub zoom: swm_config::Zoom,
    pub debug: swm_config::Debug,
    pub layer_rules: Vec<swm_config::LayerRule>,
}

impl RuntimeSettings {
    pub fn new(config: &swm_config::RuntimeConfig, applied_input: swm_config::Input) -> Self {
        Self::new_with_color_scheme(
            config,
            applied_input,
            swm_config::SystemColorScheme::NoPreference,
        )
    }

    pub fn new_with_color_scheme(
        config: &swm_config::RuntimeConfig,
        applied_input: swm_config::Input,
        system_color_scheme: swm_config::SystemColorScheme,
    ) -> Self {
        Self {
            overlays: config.overlays.resolved_for_system(system_color_scheme),
            configured_overlays: config.overlays,
            system_color_scheme,
            input: applied_input,
            animations: config.animations.clone(),
            decorations: config.decorations,
            font: config.font.clone(),
            background: config.background.clone(),
            field: config.field,
            zoom: config.field.zoom,
            debug: config.debug,
            layer_rules: config.layer_rules.clone(),
        }
    }

    pub fn visuals_changed(&self, config: &swm_config::RuntimeConfig) -> bool {
        self.decorations != config.decorations
            || self.animations != config.animations
            || self.font != config.font
            || self.background != config.background
            || self.zoom != config.field.zoom
            || self.field.pins != config.field.pins
            || self.configured_overlays != config.overlays
            || self.layer_rules != config.layer_rules
    }

    /// Replaces settings that do not need protocol-specific application.
    /// Input is applied separately so a failed XKB reload can retain the last
    /// working keyboard configuration.
    pub fn reload_non_input(&mut self, config: &swm_config::RuntimeConfig) {
        self.animations = config.animations.clone();
        self.decorations = config.decorations;
        self.font = config.font.clone();
        self.background = config.background.clone();
        self.field = config.field;
        self.configured_overlays = config.overlays;
        self.overlays = config
            .overlays
            .resolved_for_system(self.system_color_scheme);
        self.zoom = config.field.zoom;
        // Protocol advertisement is fixed at session startup. Preserve its
        // applied value while allowing the visual debug settings to reload.
        self.layer_rules.clone_from(&config.layer_rules);
    }

    pub fn set_system_color_scheme(&mut self, scheme: swm_config::SystemColorScheme) -> bool {
        if self.system_color_scheme == scheme {
            return false;
        }
        self.system_color_scheme = scheme;
        let overlays = self.configured_overlays.resolved_for_system(scheme);
        let changed = self.overlays != overlays;
        self.overlays = overlays;
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::RuntimeSettings;

    #[test]
    fn reload_preserves_explicit_sync_policy_until_a_new_session() {
        for initially_disabled in [false, true] {
            let mut config = swm_config::RuntimeConfig::default();
            config.debug.disable_explicit_sync = initially_disabled;
            let mut settings = RuntimeSettings::new(&config, config.input.clone());
            config.debug.disable_explicit_sync = !initially_disabled;

            assert!(!settings.visuals_changed(&config));
            settings.reload_non_input(&config);
            assert_eq!(settings.debug.disable_explicit_sync, initially_disabled);
            assert!(!settings.visuals_changed(&config));

            let restarted = RuntimeSettings::new(&config, config.input.clone());
            assert_eq!(restarted.debug.disable_explicit_sync, !initially_disabled);
        }
    }

    #[test]
    fn system_scheme_updates_only_explicit_system_colours() {
        let mut config = swm_config::RuntimeConfig::default();
        config.overlays.background_color = swm_config::OverlayColorMode::System;
        let mut settings = RuntimeSettings::new_with_color_scheme(
            &config,
            config.input.clone(),
            swm_config::SystemColorScheme::PreferDark,
        );
        assert_eq!(
            settings.overlays.background_color,
            swm_config::OverlayColorMode::Dark
        );
        assert!(!settings.set_system_color_scheme(swm_config::SystemColorScheme::PreferDark));
        assert!(settings.set_system_color_scheme(swm_config::SystemColorScheme::PreferLight));
        assert_eq!(
            settings.overlays.background_color,
            swm_config::OverlayColorMode::Light
        );
        assert_eq!(
            settings.overlays.border_color,
            swm_config::Overlays::default().border_color
        );
    }

    #[test]
    fn visual_change_detection_ignores_non_visual_settings() {
        let config = swm_config::RuntimeConfig::default();
        let mut settings = RuntimeSettings::new(&config, config.input.clone());
        let mut changed = config.clone();
        changed.input.repeat_rate += 5;

        assert!(!settings.visuals_changed(&changed));

        changed.overlays.radius_px += 1;
        assert!(settings.visuals_changed(&changed));

        settings.reload_non_input(&changed);
        assert!(!settings.visuals_changed(&changed));

        changed.field.pins.size = 1.5;
        assert!(settings.visuals_changed(&changed));

        changed.decorations.border_width_px += 1;
        assert!(settings.visuals_changed(&changed));

        settings.reload_non_input(&changed);
        assert!(!settings.visuals_changed(&changed));
    }
}
