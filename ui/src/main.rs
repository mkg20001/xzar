use std::collections::BTreeMap;

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

const TAILWIND_CSS: &str = include_str!("../assets/tailwind.css");

fn main() {
    dioxus::launch(App);
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PinRoot {
    drv_id: String,
    drv_full: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Pin {
    id: i32,
    name: String,
    description: Option<String>,
    created: String,
    expires: Option<String>,
    abandoned: bool,
    leave_after_abandon: Option<i64>,
    roots: Vec<PinRoot>,
}

/// Format milliseconds as human-readable duration
fn format_duration(ms: i64) -> String {
    const DAY_MS: i64 = 24 * 60 * 60 * 1000;
    const WEEK_MS: i64 = 7 * DAY_MS;
    const MONTH_MS: i64 = 30 * DAY_MS;
    const YEAR_MS: i64 = 365 * DAY_MS;

    if ms >= YEAR_MS {
        let years = ms / YEAR_MS;
        format!("{}y", years)
    } else if ms >= MONTH_MS {
        let months = ms / MONTH_MS;
        format!("{}m", months)
    } else if ms >= WEEK_MS {
        let weeks = ms / WEEK_MS;
        format!("{}w", weeks)
    } else if ms >= DAY_MS {
        let days = ms / DAY_MS;
        format!("{}d", days)
    } else {
        format!("{}ms", ms)
    }
}

/// Group pins by name, with non-abandoned pins first in each group
fn group_pins(pins: &[Pin]) -> Vec<(String, Vec<Pin>)> {
    let mut groups: BTreeMap<String, Vec<Pin>> = BTreeMap::new();

    for pin in pins {
        groups.entry(pin.name.clone()).or_default().push(pin.clone());
    }

    // Sort each group: non-abandoned first
    for pins in groups.values_mut() {
        pins.sort_by_key(|p| p.abandoned);
    }

    // Convert to vec, sorted by whether the first pin is abandoned (active groups first)
    let mut result: Vec<_> = groups.into_iter().collect();
    result.sort_by_key(|(_, pins)| pins.first().is_some_and(|p| p.abandoned));
    result
}

async fn fetch_pins(server_url: &str, token: &str) -> Result<Vec<Pin>, String> {
    let client = reqwest::Client::new();
    let url = format!("{}/pins", server_url.trim_end_matches('/'));

    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Server error: {}", response.status()));
    }

    response
        .json::<Vec<Pin>>()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))
}

async fn abandon_pin(server_url: &str, token: &str, pin_id: i32) -> Result<(), String> {
    let client = reqwest::Client::new();
    let url = format!("{}/pins/{}", server_url.trim_end_matches('/'), pin_id);

    let response = client
        .delete(&url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Server error: {}", response.status()));
    }

    Ok(())
}

#[component]
fn App() -> Element {
    let mut server_url = use_signal(|| String::from("http://localhost:17788"));
    let mut token = use_signal(|| String::new());
    let mut pins = use_signal(|| Vec::<Pin>::new());
    let mut error = use_signal(|| Option::<String>::None);
    let mut loading = use_signal(|| false);
    let mut authenticated = use_signal(|| false);

    let load_pins = move |_| async move {
        loading.set(true);
        error.set(None);

        match fetch_pins(&server_url.read(), &token.read()).await {
            Ok(fetched_pins) => {
                pins.set(fetched_pins);
                authenticated.set(true);
            }
            Err(e) => {
                error.set(Some(e));
                authenticated.set(false);
            }
        }

        loading.set(false);
    };

    let refresh_pins = move |_| async move {
        loading.set(true);
        error.set(None);

        match fetch_pins(&server_url.read(), &token.read()).await {
            Ok(fetched_pins) => {
                pins.set(fetched_pins);
            }
            Err(e) => {
                error.set(Some(e));
            }
        }

        loading.set(false);
    };

    rsx! {
        document::Style { {TAILWIND_CSS} }
        div { class: "min-h-screen bg-gray-100 py-8",
            div { class: "max-w-6xl mx-auto px-4",
                h1 { class: "text-3xl font-bold text-gray-800 mb-8", "xzar Binary Cache" }

                // Login form
                if !*authenticated.read() {
                    div { class: "bg-white rounded-lg shadow-md p-6 mb-6",
                        h2 { class: "text-xl font-semibold text-gray-700 mb-4", "Connect to Server" }

                        div { class: "space-y-4",
                            div {
                                label { class: "block text-sm font-medium text-gray-700 mb-1",
                                    "Server URL"
                                }
                                input {
                                    class: "w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500",
                                    r#type: "text",
                                    placeholder: "http://localhost:17788",
                                    value: "{server_url}",
                                    oninput: move |e| server_url.set(e.value())
                                }
                            }

                            div {
                                label { class: "block text-sm font-medium text-gray-700 mb-1",
                                    "Token"
                                }
                                input {
                                    class: "w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500",
                                    r#type: "password",
                                    placeholder: "Enter your upload token",
                                    value: "{token}",
                                    oninput: move |e| token.set(e.value())
                                }
                            }

                            button {
                                class: "w-full bg-blue-600 text-white py-2 px-4 rounded-md hover:bg-blue-700 focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:opacity-50",
                                disabled: *loading.read(),
                                onclick: load_pins,
                                if *loading.read() { "Connecting..." } else { "Connect" }
                            }
                        }

                        if let Some(err) = error.read().as_ref() {
                            div { class: "mt-4 p-3 bg-red-100 text-red-700 rounded-md",
                                "{err}"
                            }
                        }
                    }
                } else {
                    // Authenticated view
                    div { class: "bg-white rounded-lg shadow-md p-6 mb-6",
                        div { class: "flex justify-between items-center mb-4",
                            h2 { class: "text-xl font-semibold text-gray-700", "Pins" }
                            div { class: "flex gap-2",
                                button {
                                    class: "bg-gray-200 text-gray-700 py-2 px-4 rounded-md hover:bg-gray-300 focus:outline-none focus:ring-2 focus:ring-gray-400",
                                    onclick: refresh_pins,
                                    "Refresh"
                                }
                                button {
                                    class: "bg-red-100 text-red-700 py-2 px-4 rounded-md hover:bg-red-200 focus:outline-none focus:ring-2 focus:ring-red-400",
                                    onclick: move |_| {
                                        authenticated.set(false);
                                        pins.set(Vec::new());
                                    },
                                    "Logout"
                                }
                            }
                        }

                        if let Some(err) = error.read().as_ref() {
                            div { class: "mb-4 p-3 bg-red-100 text-red-700 rounded-md",
                                "{err}"
                            }
                        }

                        if pins.read().is_empty() {
                            div { class: "text-gray-500 text-center py-8",
                                "No pins found"
                            }
                        } else {
                            div { class: "space-y-6",
                                for (name, group) in group_pins(&pins.read()) {
                                    PinGroup {
                                        key: "{name}",
                                        name: name,
                                        pins: group,
                                        server_url: server_url.read().clone(),
                                        token: token.read().clone(),
                                        on_refresh: move |_| async move {
                                            if let Ok(fetched_pins) = fetch_pins(&server_url.read(), &token.read()).await {
                                                pins.set(fetched_pins);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn PinGroup(
    name: String,
    pins: Vec<Pin>,
    server_url: String,
    token: String,
    on_refresh: EventHandler<()>,
) -> Element {
    let mut expanded = use_signal(|| true);
    let mut show_abandoned = use_signal(|| false);

    let active_pins: Vec<_> = pins.iter().filter(|p| !p.abandoned).cloned().collect();
    let abandoned_pins: Vec<_> = pins.iter().filter(|p| p.abandoned).cloned().collect();
    let abandoned_count = abandoned_pins.len();

    rsx! {
        div { class: "border border-gray-200 rounded-lg overflow-hidden",
            // Header - clickable to collapse/expand
            button {
                class: "w-full bg-gray-50 px-4 py-3 border-b border-gray-200 text-left hover:bg-gray-100 transition-colors",
                onclick: move |_| {
                    let current = *expanded.read();
                    expanded.set(!current);
                },
                div { class: "flex justify-between items-center",
                    div { class: "flex items-center gap-2",
                        span { class: "text-gray-400 text-sm",
                            if *expanded.read() { "▼" } else { "▶" }
                        }
                        h3 { class: "text-lg font-semibold text-gray-800", "{name}" }
                    }
                    div { class: "flex items-center gap-2",
                        span { class: "text-xs text-gray-500",
                            "{active_pins.len()} active"
                        }
                        if abandoned_count > 0 {
                            span { class: "text-xs text-gray-400",
                                "+ {abandoned_count} abandoned"
                            }
                        }
                    }
                }
            }

            // Content - collapsible
            if *expanded.read() {
                div { class: "divide-y divide-gray-100",
                    // Active pins
                    for pin in active_pins.iter() {
                        div { class: "pl-4",
                            PinCard {
                                key: "{pin.id}",
                                pin: pin.clone(),
                                server_url: server_url.clone(),
                                token: token.clone(),
                                on_abandoned: move |_| {
                                    on_refresh.call(());
                                }
                            }
                        }
                    }

                    // Show abandoned toggle
                    if abandoned_count > 0 {
                        if *show_abandoned.read() {
                            // Abandoned pins
                            for pin in abandoned_pins.iter() {
                                div { class: "pl-4",
                                    PinCard {
                                        key: "{pin.id}",
                                        pin: pin.clone(),
                                        server_url: server_url.clone(),
                                        token: token.clone(),
                                        on_abandoned: move |_| {
                                            on_refresh.call(());
                                        }
                                    }
                                }
                            }
                            button {
                                class: "w-full px-4 py-2 text-sm text-gray-500 hover:text-gray-700 hover:bg-gray-50 text-left pl-8",
                                onclick: move |e| {
                                    e.stop_propagation();
                                    show_abandoned.set(false);
                                },
                                "Hide abandoned pins"
                            }
                        } else {
                            button {
                                class: "w-full px-4 py-2 text-sm text-gray-500 hover:text-gray-700 hover:bg-gray-50 text-left pl-8",
                                onclick: move |e| {
                                    e.stop_propagation();
                                    show_abandoned.set(true);
                                },
                                if abandoned_count == 1 {
                                    "View 1 abandoned pin..."
                                } else {
                                    "View {abandoned_count} abandoned pins..."
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn PinCard(
    pin: Pin,
    server_url: String,
    token: String,
    on_abandoned: EventHandler<()>,
) -> Element {
    let mut abandoning = use_signal(|| false);
    let mut abandon_error = use_signal(|| Option::<String>::None);

    let pin_id = pin.id;
    let is_abandoned = pin.abandoned;
    let server_url_clone = server_url.clone();
    let token_clone = token.clone();

    let handle_abandon = move |_| {
        let server_url = server_url_clone.clone();
        let token = token_clone.clone();
        async move {
            abandoning.set(true);
            abandon_error.set(None);

            match abandon_pin(&server_url, &token, pin_id).await {
                Ok(()) => {
                    on_abandoned.call(());
                }
                Err(e) => {
                    abandon_error.set(Some(e));
                }
            }

            abandoning.set(false);
        }
    };

    let status_class = if pin.abandoned {
        "bg-red-100 text-red-800"
    } else if pin.expires.is_some() {
        "bg-yellow-100 text-yellow-800"
    } else {
        "bg-green-100 text-green-800"
    };

    let status_text = if pin.abandoned {
        "Abandoned"
    } else if pin.expires.is_some() {
        "Expiring"
    } else {
        "Active"
    };

    let bg_class = if pin.abandoned { "bg-gray-50" } else { "bg-white" };

    rsx! {
        div { class: "p-4 {bg_class}",
            div { class: "flex justify-between items-start mb-3",
                div {
                    if let Some(desc) = &pin.description {
                        p { class: "text-sm text-gray-500", "{desc}" }
                    }
                }
                div { class: "flex items-center gap-2",
                    span { class: "px-2 py-1 text-xs font-medium rounded-full {status_class}",
                        "{status_text}"
                    }
                    if !is_abandoned {
                        button {
                            class: "px-3 py-1 text-sm bg-red-500 text-white rounded hover:bg-red-600 disabled:opacity-50",
                            disabled: *abandoning.read(),
                            onclick: handle_abandon,
                            if *abandoning.read() { "..." } else { "Abandon" }
                        }
                    }
                }
            }

            if let Some(err) = abandon_error.read().as_ref() {
                div { class: "mb-3 p-2 bg-red-100 text-red-700 text-sm rounded",
                    "{err}"
                }
            }

            div { class: "text-sm text-gray-600 space-y-1",
                div { "ID: {pin.id}" }
                div { "Created: {pin.created}" }
                if let Some(expires) = &pin.expires {
                    div { "Expires: {expires}" }
                }
                if let Some(leave) = pin.leave_after_abandon {
                    div { "Leave after abandon: {format_duration(leave)}" }
                }
            }

            if !pin.roots.is_empty() {
                div { class: "mt-3",
                    h4 { class: "text-sm font-medium text-gray-700 mb-2",
                        "Roots ({pin.roots.len()})"
                    }
                    div { class: "space-y-1",
                        for root in pin.roots.iter() {
                            div {
                                key: "{root.drv_id}",
                                class: "text-xs font-mono bg-gray-100 p-2 rounded truncate",
                                title: "/nix/store/{root.drv_full}",
                                "/nix/store/{root.drv_full}"
                            }
                        }
                    }
                }
            }
        }
    }
}
