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
        div { class: "min-h-screen bg-gradient-to-br from-gray-50 to-gray-100 py-8",
            div { class: "max-w-6xl mx-auto px-4",
                // Header
                div { class: "text-center mb-8",
                    h1 { class: "text-3xl font-bold text-gray-800 flex items-center justify-center gap-3",
                        span { "⚡" }
                        "xzar Binary Cache"
                    }
                    p { class: "text-gray-500 mt-2", "Nix store path caching made simple" }
                }

                // Login form
                if !*authenticated.read() {
                    div { class: "max-w-md mx-auto",
                        div { class: "bg-white rounded-xl shadow-lg overflow-hidden",
                            // Header
                            div { class: "bg-gradient-to-r from-blue-600 to-blue-700 px-6 py-8 text-center",
                                div { class: "text-4xl mb-3", "🔐" }
                                h2 { class: "text-xl font-semibold text-white", "Connect to Server" }
                                p { class: "text-blue-200 text-sm mt-1", "Enter your cache server details" }
                            }

                            // Form
                            div { class: "p-6 space-y-5",
                                div {
                                    label { class: "block text-sm font-medium text-gray-700 mb-2",
                                        "Server URL"
                                    }
                                    input {
                                        class: "w-full px-4 py-3 border border-gray-200 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent transition-shadow",
                                        r#type: "text",
                                        placeholder: "http://localhost:17788",
                                        value: "{server_url}",
                                        oninput: move |e| server_url.set(e.value())
                                    }
                                }

                                div {
                                    label { class: "block text-sm font-medium text-gray-700 mb-2",
                                        "Token"
                                    }
                                    input {
                                        class: "w-full px-4 py-3 border border-gray-200 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent transition-shadow",
                                        r#type: "password",
                                        placeholder: "Enter your upload token",
                                        value: "{token}",
                                        oninput: move |e| token.set(e.value())
                                    }
                                }

                                button {
                                    class: "w-full bg-blue-600 text-white py-3 px-4 rounded-lg font-medium hover:bg-blue-700 focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-2 disabled:opacity-50 transition-colors",
                                    disabled: *loading.read(),
                                    onclick: load_pins,
                                    if *loading.read() { "⏳ Connecting..." } else { "→ Connect" }
                                }

                                if let Some(err) = error.read().as_ref() {
                                    div { class: "p-4 bg-red-50 text-red-700 rounded-lg border border-red-200 flex items-start gap-2",
                                        span { class: "flex-shrink-0", "⚠️" }
                                        span { "{err}" }
                                    }
                                }
                            }
                        }
                    }
                } else {
                    // Authenticated view
                    div { class: "bg-white rounded-xl shadow-lg overflow-hidden",
                        // Header bar
                        div { class: "bg-gradient-to-r from-blue-600 to-blue-700 px-6 py-4",
                            div { class: "flex justify-between items-center",
                                div { class: "flex items-center gap-3",
                                    span { class: "text-2xl", "📌" }
                                    h2 { class: "text-xl font-semibold text-white", "Pins" }
                                    span { class: "text-blue-200 text-sm",
                                        "({pins.read().len()} total)"
                                    }
                                }
                                div { class: "flex gap-2",
                                    button {
                                        class: "flex items-center gap-2 bg-white/20 text-white py-2 px-4 rounded-lg hover:bg-white/30 transition-colors",
                                        onclick: refresh_pins,
                                        "↻ Refresh"
                                    }
                                    button {
                                        class: "flex items-center gap-2 bg-white/10 text-white/80 py-2 px-4 rounded-lg hover:bg-white/20 transition-colors",
                                        onclick: move |_| {
                                            authenticated.set(false);
                                            pins.set(Vec::new());
                                        },
                                        "Logout"
                                    }
                                }
                            }
                        }

                        // Content area
                        div { class: "p-6",
                            if let Some(err) = error.read().as_ref() {
                                div { class: "mb-4 p-4 bg-red-50 text-red-700 rounded-lg border border-red-200 flex items-center gap-2",
                                    span { "⚠️" }
                                    "{err}"
                                }
                            }

                            if pins.read().is_empty() {
                                div { class: "text-center py-12",
                                    div { class: "text-4xl mb-4", "📭" }
                                    p { class: "text-gray-500 text-lg", "No pins found" }
                                    p { class: "text-gray-400 text-sm mt-1", "Upload some paths to create your first pin" }
                                }
                            } else {
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

    rsx! {
        div { class: if depth > 0 { "pl-5 relative" } else { "" },
            // Vertical connector line for nested items
            if depth > 0 {
                div { class: "absolute left-2 top-0 bottom-0 w-px bg-gray-200" }
            }

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
                    PinLeaf {
                        key: "{pin.id}",
                        pin: pin.clone(),
                        depth: depth,
                        server_url: server_url.clone(),
                        token: token.clone(),
                        on_abandoned: move |_| {
                            on_refresh.call(());
                        }
                    }
                }

                // Abandoned pins toggle
                if !abandoned_pins.is_empty() {
                    if *show_abandoned.read() {
                        for pin in abandoned_pins.iter() {
                            PinLeaf {
                                key: "{pin.id}",
                                pin: pin.clone(),
                                depth: depth,
                                server_url: server_url.clone(),
                                token: token.clone(),
                                on_abandoned: move |_| {
                                    on_refresh.call(());
                                }
                            }
                        }
                        button {
                            class: "flex items-center gap-2 ml-1 text-xs text-gray-400 hover:text-gray-600 py-1.5 transition-colors",
                            onclick: move |_| show_abandoned.set(false),
                            span { class: "text-gray-300", "└" }
                            "Hide abandoned"
                        }
                    } else {
                        button {
                            class: "flex items-center gap-2 ml-1 text-xs text-gray-400 hover:text-gray-600 py-1.5 transition-colors",
                            onclick: move |_| show_abandoned.set(true),
                            span { class: "text-gray-300", "└" }
                            span { class: "opacity-60", "📦" }
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

    let active_count = child.active_pins();
    let abandoned_count = child.abandoned_pins();
    let has_active = active_count > 0;

    rsx! {
        div { class: "relative",
            // Directory header
            button {
                class: "group w-full flex items-center gap-2 py-1.5 text-left rounded-md hover:bg-blue-50 transition-all duration-150",
                onclick: move |_| {
                    let current = *expanded.read();
                    expanded.set(!current);
                },
                // Expand/collapse chevron
                span { class: "text-gray-400 group-hover:text-blue-500 transition-colors w-4 text-center",
                    if *expanded.read() { "▾" } else { "▸" }
                }
                // Folder icon
                span { class: "text-base",
                    if *expanded.read() { "📂" } else { "📁" }
                }
                // Directory name
                span { class: "font-medium text-gray-700 group-hover:text-blue-700 transition-colors",
                    "{name}"
                }
                // Pin counts badge
                div { class: "flex items-center gap-1.5 ml-auto pr-2",
                    if has_active {
                        span { class: "inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium bg-green-100 text-green-700",
                            "{active_count}"
                        }
                    }
                    if abandoned_count > 0 {
                        span { class: "inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium bg-gray-100 text-gray-500",
                            "+{abandoned_count}"
                        }
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
    depth: usize,
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

    let (status_class, status_icon) = if pin.abandoned {
        ("bg-red-50 text-red-600 border-red-200", "🗑️")
    } else if pin.expires.is_some() {
        ("bg-amber-50 text-amber-600 border-amber-200", "⏳")
    } else {
        ("bg-emerald-50 text-emerald-600 border-emerald-200", "✓")
    };

    let card_class = if pin.abandoned {
        "bg-gray-50 border border-gray-200 opacity-60"
    } else {
        "bg-white border border-gray-200 shadow-sm hover:shadow-md hover:border-blue-200"
    };

    // Get the leaf name (last part of the path)
    let leaf_name = pin.name.split('/').last().unwrap_or(&pin.name);

    rsx! {
        div { class: "relative py-1",
            // Horizontal connector
            if depth > 0 {
                div { class: "absolute left-[-12px] top-1/2 w-3 h-px bg-gray-200" }
            }

            div { class: "{card_class} rounded-lg transition-all duration-150",
                // Header row
                button {
                    class: "w-full flex items-center gap-3 px-4 py-3 text-left",
                    onclick: move |_| {
                        let current = *expanded.read();
                        expanded.set(!current);
                    },
                    // Pin icon
                    span { class: "text-base flex-shrink-0",
                        if pin.abandoned { "📦" } else { "📌" }
                    }
                    // Pin name
                    div { class: "flex-grow min-w-0",
                        span { class: "font-medium text-gray-800 block truncate", "{leaf_name}" }
                        span { class: "text-xs text-gray-400",
                            "{pin.roots.len()} root"
                            if pin.roots.len() != 1 { "s" }
                        }
                    }
                    // Status badge
                    span { class: "inline-flex items-center gap-1 px-2.5 py-1 rounded-full text-xs font-medium border {status_class} flex-shrink-0",
                        "{status_icon}"
                    }
                    // Expand indicator
                    span { class: "text-gray-300 text-sm flex-shrink-0 ml-1",
                        if *expanded.read() { "▾" } else { "▸" }
                    }
                }

                // Expanded details
                if *expanded.read() {
                    div { class: "px-4 pb-4 pt-2 border-t border-gray-100",
                        if let Some(err) = abandon_error.read().as_ref() {
                            div { class: "mb-3 p-3 bg-red-50 text-red-700 text-sm rounded-lg border border-red-200",
                                "{err}"
                            }
                        }

                        if let Some(desc) = &pin.description {
                            p { class: "text-gray-600 mb-3 text-sm", "{desc}" }
                        }

                        // Metadata grid
                        div { class: "grid grid-cols-2 gap-x-4 gap-y-2 text-sm mb-3",
                            div { class: "text-gray-400", "ID" }
                            div { class: "text-gray-700 font-mono text-xs", "{pin.id}" }
                            div { class: "text-gray-400", "Created" }
                            div { class: "text-gray-700", "{pin.created}" }
                            if let Some(expires) = &pin.expires {
                                div { class: "text-gray-400", "Expires" }
                                div { class: "text-amber-600", "{expires}" }
                            }
                            if let Some(leave) = pin.leave_after_abandon {
                                div { class: "text-gray-400", "Leave after abandon" }
                                div { class: "text-gray-700", "{format_duration(leave)}" }
                            }
                        }

                        // Roots section
                        if !pin.roots.is_empty() {
                            details { class: "group",
                                summary { class: "cursor-pointer text-sm text-gray-500 hover:text-gray-700 py-1 select-none",
                                    "📦 {pin.roots.len()} store path"
                                    if pin.roots.len() != 1 { "s" }
                                }
                                div { class: "mt-2 space-y-1.5",
                                    for root in pin.roots.iter() {
                                        div {
                                            key: "{root.drv_id}",
                                            class: "text-xs font-mono bg-gray-50 p-2 rounded-md border border-gray-100 truncate text-gray-600",
                                            title: "/nix/store/{root.drv_full}",
                                            "/nix/store/{root.drv_full}"
                                        }
                                    }
                                }
                            }
                        }

                        // Abandon button
                        if !is_abandoned {
                            div { class: "mt-4 pt-3 border-t border-gray-100",
                                button {
                                    class: "px-4 py-2 text-sm bg-red-50 text-red-600 rounded-lg hover:bg-red-100 border border-red-200 transition-colors disabled:opacity-50",
                                    disabled: *abandoning.read(),
                                    onclick: handle_abandon,
                                    if *abandoning.read() { "Abandoning..." } else { "🗑️ Abandon Pin" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
