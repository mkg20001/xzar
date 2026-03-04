use std::collections::BTreeMap;

use dioxus::prelude::*;
use xzar_common::{PinResponse as Pin, format_duration};

const TAILWIND_CSS: &str = include_str!("../assets/tailwind.css");

fn main() {
    dioxus::launch(App);
}

/// A node in the pin tree structure
#[derive(Debug, Clone, Default, PartialEq)]
struct PinTreeNode {
    /// Pins at this exact path
    pins: Vec<Pin>,
    /// Child nodes (path segment -> child)
    children: BTreeMap<String, PinTreeNode>,
}

impl PinTreeNode {
    /// Insert a pin into the tree based on its name (split by /)
    fn insert(&mut self, pin: Pin) {
        let parts: Vec<String> = pin.name.split('/').map(|s| s.to_string()).collect();
        let parts_ref: Vec<&str> = parts.iter().map(|s| s.as_str()).collect();
        self.insert_at_path(&parts_ref, pin);
    }

    fn insert_at_path(&mut self, path: &[&str], pin: Pin) {
        if path.is_empty() || (path.len() == 1 && path[0].is_empty()) {
            self.pins.push(pin);
        } else if path.len() == 1 {
            // Leaf node - this is the final segment
            self.children
                .entry(path[0].to_string())
                .or_default()
                .pins
                .push(pin);
        } else {
            // Intermediate node
            self.children
                .entry(path[0].to_string())
                .or_default()
                .insert_at_path(&path[1..], pin);
        }
    }

    /// Check if this node has any active (non-abandoned) pins in its subtree
    fn has_active_pins(&self) -> bool {
        self.pins.iter().any(|p| !p.abandoned)
            || self.children.values().any(|c| c.has_active_pins())
    }

    /// Count total pins in subtree
    #[allow(dead_code)]
    fn total_pins(&self) -> usize {
        self.pins.len() + self.children.values().map(|c| c.total_pins()).sum::<usize>()
    }

    /// Count active pins in subtree
    fn active_pins(&self) -> usize {
        self.pins.iter().filter(|p| !p.abandoned).count()
            + self.children.values().map(|c| c.active_pins()).sum::<usize>()
    }

    /// Count abandoned pins in subtree
    fn abandoned_pins(&self) -> usize {
        self.pins.iter().filter(|p| p.abandoned).count()
            + self.children.values().map(|c| c.abandoned_pins()).sum::<usize>()
    }
}

/// Build a tree from a list of pins
fn build_pin_tree(pins: &[Pin]) -> PinTreeNode {
    let mut root = PinTreeNode::default();
    for pin in pins {
        root.insert(pin.clone());
    }
    root
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
                            div { class: "space-y-1",
                                TreeNodeView {
                                    node: build_pin_tree(&pins.read()),
                                    path: String::new(),
                                    depth: 0,
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

#[component]
fn TreeNodeView(
    node: PinTreeNode,
    path: String,
    depth: usize,
    server_url: String,
    token: String,
    on_refresh: EventHandler<()>,
) -> Element {
    let mut show_abandoned = use_signal(|| false);

    let has_pins = !node.pins.is_empty();

    let active_pins: Vec<_> = node.pins.iter().filter(|p| !p.abandoned).cloned().collect();
    let abandoned_pins: Vec<_> = node.pins.iter().filter(|p| p.abandoned).cloned().collect();

    // Sort children: those with active pins first
    let mut sorted_children: Vec<_> = node.children.into_iter().collect();
    sorted_children.sort_by_key(|(_, child)| !child.has_active_pins());

    let indent_class = match depth {
        0 => "",
        1 => "ml-4",
        2 => "ml-8",
        3 => "ml-12",
        _ => "ml-16",
    };

    rsx! {
        div { class: "{indent_class}",
            // Render children (directories)
            for (name, child) in sorted_children.iter() {
                TreeDirNode {
                    key: "{name}",
                    name: name.clone(),
                    child: child.clone(),
                    path: path.clone(),
                    depth: depth,
                    server_url: server_url.clone(),
                    token: token.clone(),
                    on_refresh: on_refresh.clone()
                }
            }

            // Render pins at this level (leaves)
            if has_pins {
                // Active pins
                for pin in active_pins.iter() {
                    div { class: "border-l-2 border-green-300 my-1",
                        PinLeaf {
                            pin: pin.clone(),
                            server_url: server_url.clone(),
                            token: token.clone(),
                            on_abandoned: move |_| {
                                on_refresh.call(());
                            }
                        }
                    }
                }

                // Abandoned pins toggle
                if !abandoned_pins.is_empty() {
                    if *show_abandoned.read() {
                        for pin in abandoned_pins.iter() {
                            div { class: "border-l-2 border-gray-300 my-1",
                                PinLeaf {
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
                            class: "text-xs text-gray-400 hover:text-gray-600 px-3 py-1",
                            onclick: move |_| show_abandoned.set(false),
                            "Hide abandoned"
                        }
                    } else {
                        button {
                            class: "text-xs text-gray-400 hover:text-gray-600 px-3 py-1",
                            onclick: move |_| show_abandoned.set(true),
                            "Show {abandoned_pins.len()} abandoned..."
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn TreeDirNode(
    name: String,
    child: PinTreeNode,
    path: String,
    depth: usize,
    server_url: String,
    token: String,
    on_refresh: EventHandler<()>,
) -> Element {
    let mut expanded = use_signal(|| true);

    rsx! {
        div { class: "border-l-2 border-gray-200 my-1",
            // Directory header
            button {
                class: "w-full flex items-center gap-2 px-3 py-2 text-left hover:bg-gray-50 rounded-r transition-colors",
                onclick: move |_| {
                    let current = *expanded.read();
                    expanded.set(!current);
                },
                span { class: "text-gray-400 text-xs w-4",
                    if *expanded.read() { "▼" } else { "▶" }
                }
                span { class: "font-medium text-gray-700", "{name}/" }
                span { class: "text-xs text-gray-400 ml-auto",
                    if child.active_pins() > 0 {
                        "{child.active_pins()} active"
                    }
                    if child.abandoned_pins() > 0 {
                        " +{child.abandoned_pins()} abandoned"
                    }
                }
            }

            // Child content
            if *expanded.read() {
                TreeNodeView {
                    node: child.clone(),
                    path: if path.is_empty() { name.clone() } else { format!("{}/{}", path, name) },
                    depth: depth + 1,
                    server_url: server_url.clone(),
                    token: token.clone(),
                    on_refresh: on_refresh.clone()
                }
            }
        }
    }
}

#[component]
fn PinLeaf(
    pin: Pin,
    server_url: String,
    token: String,
    on_abandoned: EventHandler<()>,
) -> Element {
    let mut expanded = use_signal(|| false);
    let mut abandoning = use_signal(|| false);
    let mut abandon_error = use_signal(|| Option::<String>::None);

    let pin_id = pin.id;
    let is_abandoned = pin.abandoned;
    let server_url_clone = server_url.clone();
    let token_clone = token.clone();

    let handle_abandon = move |e: Event<MouseData>| {
        e.stop_propagation();
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

    // Get the leaf name (last part of the path)
    let leaf_name = pin.name.split('/').last().unwrap_or(&pin.name);

    rsx! {
        div { class: "{bg_class} rounded-r",
            // Header row
            button {
                class: "w-full flex items-center gap-2 px-3 py-2 text-left hover:bg-gray-50 transition-colors",
                onclick: move |_| {
                    let current = *expanded.read();
                    expanded.set(!current);
                },
                span { class: "text-gray-400 text-xs w-4",
                    if *expanded.read() { "▼" } else { "▶" }
                }
                span { class: "font-medium text-gray-800", "{leaf_name}" }
                span { class: "px-2 py-0.5 text-xs font-medium rounded-full {status_class} ml-2",
                    "{status_text}"
                }
                span { class: "text-xs text-gray-400 ml-auto",
                    "{pin.roots.len()} roots"
                }
                if !is_abandoned {
                    button {
                        class: "px-2 py-1 text-xs bg-red-500 text-white rounded hover:bg-red-600 disabled:opacity-50 ml-2",
                        disabled: *abandoning.read(),
                        onclick: handle_abandon,
                        if *abandoning.read() { "..." } else { "Abandon" }
                    }
                }
            }

            // Expanded details
            if *expanded.read() {
                div { class: "px-3 py-2 ml-6 text-sm border-t border-gray-100",
                    if let Some(err) = abandon_error.read().as_ref() {
                        div { class: "mb-2 p-2 bg-red-100 text-red-700 text-xs rounded",
                            "{err}"
                        }
                    }

                    if let Some(desc) = &pin.description {
                        p { class: "text-gray-500 mb-2", "{desc}" }
                    }

                    div { class: "text-xs text-gray-500 space-y-1",
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
                        div { class: "mt-2",
                            h4 { class: "text-xs font-medium text-gray-600 mb-1",
                                "Roots:"
                            }
                            div { class: "space-y-1",
                                for root in pin.roots.iter() {
                                    div {
                                        key: "{root.drv_id}",
                                        class: "text-xs font-mono bg-gray-100 p-1.5 rounded truncate",
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
    }
}
