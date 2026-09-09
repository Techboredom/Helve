use leptos::prelude::*;

const STORAGE_KEY: &str = "helve-theme";

/// Reads the persisted theme (defaulting to dark) and keeps the document's
/// `data-theme` attribute — which `style.css` themes off — and localStorage
/// in sync with the returned signal from then on.
pub fn init_theme() -> RwSignal<String> {
    let stored = web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(STORAGE_KEY).ok().flatten())
        .filter(|v| v == "light" || v == "dark")
        .unwrap_or_else(|| "dark".to_string());
    let theme = RwSignal::new(stored);

    Effect::new(move |_| {
        let value = theme.get();
        if let Some(el) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.document_element()) {
            let _ = el.set_attribute("data-theme", &value);
        }
        if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
            let _ = storage.set_item(STORAGE_KEY, &value);
        }
    });

    theme
}

pub fn toggle(theme: RwSignal<String>) {
    let next = if theme.get() == "light" { "dark" } else { "light" };
    theme.set(next.to_string());
}
