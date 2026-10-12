use slint_build::{CompilerConfiguration, DefaultTranslationContext};

fn main() {
    let config = CompilerConfiguration::new()
        .with_style("fluent-dark".into())
        // Translations live in ui/translations/<lang>/LC_MESSAGES/klipper-helm.po and are compiled in.
        // No per-component context, so one msgid is translated the same way everywhere.
        .with_bundled_translations("ui/translations")
        .with_default_translation_context(DefaultTranslationContext::None);
    slint_build::compile_with_config("ui/app.slint", config).unwrap();
}
