mod common;

use std::collections::BTreeMap;

use common::InlineEdit;
use dioxus::prelude::*;
use xzar_common::{
    format_duration, AdminTokenResponse, AdminUserResponse, CreateTokenResponse,
    PinResponse as Pin, SelfResponse,
};

const TAILWIND_CSS: &str = include_str!("../assets/tailwind.css");

// ============ Global Signals ============

static SERVER_URL: GlobalSignal<String> = Signal::global(|| String::from("http://localhost:17788"));
static TOKEN: GlobalSignal<String> = Signal::global(|| String::new());

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

// ============ Self/Auth Info API ============

async fn fetch_self(server_url: &str, token: &str) -> Result<SelfResponse, String> {
    let client = reqwest::Client::new();
    let url = format!("{}/self", server_url.trim_end_matches('/'));

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
        .json::<SelfResponse>()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))
}

// ============ Admin Users API ============

async fn fetch_users(server_url: &str, token: &str) -> Result<Vec<AdminUserResponse>, String> {
    let client = reqwest::Client::new();
    let url = format!("{}/admin/users", server_url.trim_end_matches('/'));

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
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))
}

async fn create_user(server_url: &str, token: &str, name: &str, is_admin: bool) -> Result<AdminUserResponse, String> {
    let client = reqwest::Client::new();
    let url = format!("{}/admin/users", server_url.trim_end_matches('/'));

    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", token))
        .json(&serde_json::json!({ "name": name, "isAdmin": is_admin }))
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Server error: {}", response.status()));
    }

    response
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))
}

/// Update user with PATCH - only specified fields are updated
async fn update_user(
    server_url: &str,
    token: &str,
    user_id: i32,
    is_admin: Option<bool>,
    email: Option<Option<String>>,
) -> Result<(), String> {
    let client = reqwest::Client::new();
    let url = format!("{}/admin/users/{}", server_url.trim_end_matches('/'), user_id);

    // Build request body with only specified fields
    let mut body = serde_json::Map::new();
    if let Some(is_admin) = is_admin {
        body.insert("isAdmin".to_string(), serde_json::json!(is_admin));
    }
    if let Some(email) = email {
        body.insert("email".to_string(), serde_json::json!(email));
    }

    let response = client
        .patch(&url)
        .header("Authorization", format!("Bearer {}", token))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Server error: {}", response.status()));
    }

    Ok(())
}

async fn delete_user(server_url: &str, token: &str, user_id: i32) -> Result<(), String> {
    let client = reqwest::Client::new();
    let url = format!("{}/admin/users/{}", server_url.trim_end_matches('/'), user_id);

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

// ============ Admin Tokens API ============

async fn fetch_tokens(server_url: &str, token: &str) -> Result<Vec<AdminTokenResponse>, String> {
    let client = reqwest::Client::new();
    let url = format!("{}/admin/tokens", server_url.trim_end_matches('/'));

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
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))
}

async fn create_token(server_url: &str, token: &str, user_id: Option<i32>, description: Option<&str>) -> Result<CreateTokenResponse, String> {
    let client = reqwest::Client::new();
    let url = format!("{}/admin/tokens", server_url.trim_end_matches('/'));

    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", token))
        .json(&serde_json::json!({ "userId": user_id, "description": description }))
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Server error: {}", response.status()));
    }

    response
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))
}

async fn update_token(server_url: &str, token: &str, token_id: i32, description: Option<&str>) -> Result<(), String> {
    let client = reqwest::Client::new();
    let url = format!("{}/admin/tokens/{}", server_url.trim_end_matches('/'), token_id);

    let response = client
        .put(&url)
        .header("Authorization", format!("Bearer {}", token))
        .json(&serde_json::json!({ "description": description }))
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Server error: {}", response.status()));
    }

    Ok(())
}

async fn delete_token(server_url: &str, token: &str, token_id: i32) -> Result<(), String> {
    let client = reqwest::Client::new();
    let url = format!("{}/admin/tokens/{}", server_url.trim_end_matches('/'), token_id);

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

#[derive(Clone, Copy, PartialEq)]
enum ActiveTab {
    Pins,
    Admin,
}

#[component]
fn App() -> Element {
    let mut pins = use_signal(|| Vec::<Pin>::new());
    let mut error = use_signal(|| Option::<String>::None);
    let mut loading = use_signal(|| false);
    let mut authenticated = use_signal(|| false);
    let mut self_info = use_signal(|| Option::<SelfResponse>::None);
    let mut active_tab = use_signal(|| ActiveTab::Pins);

    let load_pins = move |_| async move {
        loading.set(true);
        error.set(None);

        let server_url = SERVER_URL.read().clone();
        let token = TOKEN.read().clone();

        // First fetch self info
        match fetch_self(&server_url, &token).await {
            Ok(info) => {
                self_info.set(Some(info));
            }
            Err(e) => {
                error.set(Some(e));
                authenticated.set(false);
                loading.set(false);
                return;
            }
        }

        // Then fetch pins
        match fetch_pins(&server_url, &token).await {
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

        let server_url = SERVER_URL.read().clone();
        let token = TOKEN.read().clone();

        match fetch_pins(&server_url, &token).await {
            Ok(fetched_pins) => {
                pins.set(fetched_pins);
            }
            Err(e) => {
                error.set(Some(e));
            }
        }

        loading.set(false);
    };

    let is_admin = self_info.read().as_ref().map(|s| s.is_admin).unwrap_or(false);

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
                                        value: "{SERVER_URL}",
                                        oninput: move |e| *SERVER_URL.write() = e.value()
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
                                        value: "{TOKEN}",
                                        oninput: move |e| *TOKEN.write() = e.value()
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
                                // Tabs
                                div { class: "flex items-center gap-1",
                                    button {
                                        class: if *active_tab.read() == ActiveTab::Pins {
                                            "flex items-center gap-2 bg-white/30 text-white py-2 px-4 rounded-lg font-medium"
                                        } else {
                                            "flex items-center gap-2 bg-white/10 text-white/80 py-2 px-4 rounded-lg hover:bg-white/20 transition-colors"
                                        },
                                        onclick: move |_| active_tab.set(ActiveTab::Pins),
                                        span { "📌" }
                                        "Pins"
                                        span { class: "text-xs opacity-75 ml-1",
                                            "({pins.read().len()})"
                                        }
                                    }
                                    if is_admin {
                                        button {
                                            class: if *active_tab.read() == ActiveTab::Admin {
                                                "flex items-center gap-2 bg-white/30 text-white py-2 px-4 rounded-lg font-medium"
                                            } else {
                                                "flex items-center gap-2 bg-white/10 text-white/80 py-2 px-4 rounded-lg hover:bg-white/20 transition-colors"
                                            },
                                            onclick: move |_| active_tab.set(ActiveTab::Admin),
                                            span { "⚙️" }
                                            "Admin"
                                        }
                                    }
                                }
                                // Actions
                                div { class: "flex gap-2",
                                    if *active_tab.read() == ActiveTab::Pins {
                                        button {
                                            class: "flex items-center gap-2 bg-white/20 text-white py-2 px-4 rounded-lg hover:bg-white/30 transition-colors",
                                            onclick: refresh_pins,
                                            "↻ Refresh"
                                        }
                                    }
                                    button {
                                        class: "flex items-center gap-2 bg-white/10 text-white/80 py-2 px-4 rounded-lg hover:bg-white/20 transition-colors",
                                        onclick: move |_| {
                                            authenticated.set(false);
                                            pins.set(Vec::new());
                                            self_info.set(None);
                                            active_tab.set(ActiveTab::Pins);
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

                            match *active_tab.read() {
                                ActiveTab::Pins => rsx! {
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
                                            on_refresh: move |_| async move {
                                                let server_url = SERVER_URL.read().clone();
                                                let token = TOKEN.read().clone();
                                                if let Ok(fetched_pins) = fetch_pins(&server_url, &token).await {
                                                    pins.set(fetched_pins);
                                                }
                                            }
                                        }
                                    }
                                },
                                ActiveTab::Admin => rsx! {
                                    AdminPanel {}
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
    on_abandoned: EventHandler<()>,
) -> Element {
    let mut expanded = use_signal(|| false);
    let mut abandoning = use_signal(|| false);
    let mut abandon_error = use_signal(|| Option::<String>::None);

    let pin_id = pin.id;
    let is_abandoned = pin.abandoned;

    let handle_abandon = move |e: Event<MouseData>| {
        e.stop_propagation();
        async move {
            abandoning.set(true);
            abandon_error.set(None);

            let server_url = SERVER_URL.read().clone();
            let token = TOKEN.read().clone();

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

// ============ Admin Panel Components ============

#[derive(Clone, Copy, PartialEq)]
enum AdminTab {
    Users,
    Tokens,
}

#[component]
fn AdminPanel() -> Element {
    let mut admin_tab = use_signal(|| AdminTab::Users);
    let mut users = use_signal(|| Vec::<AdminUserResponse>::new());
    let mut tokens_list = use_signal(|| Vec::<AdminTokenResponse>::new());
    let mut loading = use_signal(|| false);
    let error = use_signal(|| Option::<String>::None);

    // Load data on mount
    use_effect(move || {
        spawn(async move {
            loading.set(true);
            let server_url = SERVER_URL.read().clone();
            let token = TOKEN.read().clone();
            if let Ok(u) = fetch_users(&server_url, &token).await {
                users.set(u);
            }
            if let Ok(t) = fetch_tokens(&server_url, &token).await {
                tokens_list.set(t);
            }
            loading.set(false);
        });
    });

    rsx! {
        div {
            // Sub-tabs
            div { class: "flex gap-2 mb-6 border-b border-gray-200 pb-4",
                button {
                    class: if *admin_tab.read() == AdminTab::Users {
                        "px-4 py-2 text-sm font-medium text-blue-600 bg-blue-50 rounded-lg"
                    } else {
                        "px-4 py-2 text-sm font-medium text-gray-600 hover:bg-gray-50 rounded-lg"
                    },
                    onclick: move |_| admin_tab.set(AdminTab::Users),
                    "👥 Users ({users.read().len()})"
                }
                button {
                    class: if *admin_tab.read() == AdminTab::Tokens {
                        "px-4 py-2 text-sm font-medium text-blue-600 bg-blue-50 rounded-lg"
                    } else {
                        "px-4 py-2 text-sm font-medium text-gray-600 hover:bg-gray-50 rounded-lg"
                    },
                    onclick: move |_| admin_tab.set(AdminTab::Tokens),
                    "🔑 Tokens ({tokens_list.read().len()})"
                }
            }

            if let Some(err) = error.read().as_ref() {
                div { class: "mb-4 p-4 bg-red-50 text-red-700 rounded-lg border border-red-200",
                    "{err}"
                }
            }

            if *loading.read() {
                div { class: "text-center py-8 text-gray-500",
                    "Loading..."
                }
            } else {
                match *admin_tab.read() {
                    AdminTab::Users => rsx! {
                        UsersPanel {
                            users: users.read().clone(),
                            on_refresh: move |_| {
                                spawn(async move {
                                    let server_url = SERVER_URL.read().clone();
                                    let token = TOKEN.read().clone();
                                    if let Ok(u) = fetch_users(&server_url, &token).await {
                                        users.set(u);
                                    }
                                });
                            }
                        }
                    },
                    AdminTab::Tokens => rsx! {
                        TokensPanel {
                            tokens: tokens_list.read().clone(),
                            users: users.read().clone(),
                            on_refresh: move |_| {
                                spawn(async move {
                                    let server_url = SERVER_URL.read().clone();
                                    let token = TOKEN.read().clone();
                                    if let Ok(t) = fetch_tokens(&server_url, &token).await {
                                        tokens_list.set(t);
                                    }
                                });
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn UsersPanel(
    users: Vec<AdminUserResponse>,
    on_refresh: EventHandler<()>,
) -> Element {
    let mut show_create = use_signal(|| false);
    let mut new_name = use_signal(|| String::new());
    let mut new_is_admin = use_signal(|| false);
    let mut creating = use_signal(|| false);
    let mut create_error = use_signal(|| Option::<String>::None);

    let handle_create = move |_| {
        let name = new_name.read().clone();
        let is_admin = *new_is_admin.read();
        async move {
            creating.set(true);
            create_error.set(None);

            let server_url = SERVER_URL.read().clone();
            let token = TOKEN.read().clone();

            match create_user(&server_url, &token, &name, is_admin).await {
                Ok(_) => {
                    show_create.set(false);
                    new_name.set(String::new());
                    new_is_admin.set(false);
                    on_refresh.call(());
                }
                Err(e) => create_error.set(Some(e)),
            }

            creating.set(false);
        }
    };

    rsx! {
        div {
            // Create button
            div { class: "mb-4",
                if *show_create.read() {
                    div { class: "p-4 bg-gray-50 rounded-lg border border-gray-200",
                        h3 { class: "font-medium text-gray-800 mb-3", "Create User" }
                        if let Some(err) = create_error.read().as_ref() {
                            div { class: "mb-3 p-3 bg-red-50 text-red-700 text-sm rounded border border-red-200",
                                "{err}"
                            }
                        }
                        div { class: "space-y-3",
                            input {
                                class: "w-full px-3 py-2 border border-gray-200 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500",
                                r#type: "text",
                                placeholder: "Username",
                                value: "{new_name}",
                                oninput: move |e| new_name.set(e.value())
                            }
                            label { class: "flex items-center gap-2 text-sm text-gray-700",
                                input {
                                    r#type: "checkbox",
                                    checked: *new_is_admin.read(),
                                    onchange: move |e| new_is_admin.set(e.checked())
                                }
                                "Admin"
                            }
                            div { class: "flex gap-2",
                                button {
                                    class: "px-4 py-2 text-sm bg-blue-600 text-white rounded-lg hover:bg-blue-700 disabled:opacity-50",
                                    disabled: *creating.read() || new_name.read().is_empty(),
                                    onclick: handle_create,
                                    if *creating.read() { "Creating..." } else { "Create" }
                                }
                                button {
                                    class: "px-4 py-2 text-sm bg-gray-100 text-gray-700 rounded-lg hover:bg-gray-200",
                                    onclick: move |_| show_create.set(false),
                                    "Cancel"
                                }
                            }
                        }
                    }
                } else {
                    button {
                        class: "px-4 py-2 text-sm bg-blue-600 text-white rounded-lg hover:bg-blue-700",
                        onclick: move |_| show_create.set(true),
                        "+ Create User"
                    }
                }
            }

            // Users table
            if users.is_empty() {
                div { class: "text-center py-8 text-gray-500",
                    "No users found"
                }
            } else {
                div { class: "overflow-x-auto",
                    table { class: "w-full text-sm",
                        thead { class: "bg-gray-50",
                            tr {
                                th { class: "px-4 py-3 text-left text-gray-600 font-medium", "ID" }
                                th { class: "px-4 py-3 text-left text-gray-600 font-medium", "Name" }
                                th { class: "px-4 py-3 text-left text-gray-600 font-medium", "Email" }
                                th { class: "px-4 py-3 text-left text-gray-600 font-medium", "Admin" }
                                th { class: "px-4 py-3 text-left text-gray-600 font-medium", "Created" }
                                th { class: "px-4 py-3 text-left text-gray-600 font-medium", "Actions" }
                            }
                        }
                        tbody {
                            for user in users.iter() {
                                UserRow {
                                    key: "{user.id}",
                                    user: user.clone(),
                                    on_refresh: on_refresh.clone()
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
fn UserRow(
    user: AdminUserResponse,
    on_refresh: EventHandler<()>,
) -> Element {
    let mut deleting = use_signal(|| false);
    let mut toggling = use_signal(|| false);

    let user_id = user.id;
    let is_admin = user.is_admin;

    let handle_toggle = move |_| {
        async move {
            toggling.set(true);
            let server_url = SERVER_URL.read().clone();
            let token = TOKEN.read().clone();
            if update_user(&server_url, &token, user_id, Some(!is_admin), None).await.is_ok() {
                on_refresh.call(());
            }
            toggling.set(false);
        }
    };

    let handle_delete = move |_| {
        async move {
            deleting.set(true);
            let server_url = SERVER_URL.read().clone();
            let token = TOKEN.read().clone();
            if delete_user(&server_url, &token, user_id).await.is_ok() {
                on_refresh.call(());
            }
            deleting.set(false);
        }
    };

    rsx! {
        tr { class: "border-t border-gray-100 hover:bg-gray-50",
            td { class: "px-4 py-3 text-gray-500 font-mono text-xs", "{user.id}" }
            td { class: "px-4 py-3 font-medium text-gray-800", "{user.name}" }
            td { class: "px-4 py-3",
                InlineEdit {
                    value: user.email.clone(),
                    placeholder: "-",
                    on_save: move |new_email: Option<String>| {
                        spawn(async move {
                            let server_url = SERVER_URL.read().clone();
                            let token = TOKEN.read().clone();
                            if update_user(&server_url, &token, user_id, None, Some(new_email)).await.is_ok() {
                                on_refresh.call(());
                            }
                        });
                    }
                }
            }
            td { class: "px-4 py-3",
                if user.is_admin {
                    span { class: "px-2 py-1 text-xs font-medium bg-purple-100 text-purple-700 rounded", "Admin" }
                } else {
                    span { class: "px-2 py-1 text-xs font-medium bg-gray-100 text-gray-600 rounded", "User" }
                }
            }
            td { class: "px-4 py-3 text-gray-500 text-xs", "{user.created}" }
            td { class: "px-4 py-3",
                div { class: "flex gap-2",
                    button {
                        class: "px-3 py-1 text-xs bg-gray-100 text-gray-700 rounded hover:bg-gray-200 disabled:opacity-50",
                        disabled: *toggling.read(),
                        onclick: handle_toggle,
                        if user.is_admin { "Demote" } else { "Promote" }
                    }
                    button {
                        class: "px-3 py-1 text-xs bg-red-50 text-red-600 rounded hover:bg-red-100 disabled:opacity-50",
                        disabled: *deleting.read(),
                        onclick: handle_delete,
                        "Delete"
                    }
                }
            }
        }
    }
}

#[component]
fn TokensPanel(
    tokens: Vec<AdminTokenResponse>,
    users: Vec<AdminUserResponse>,
    on_refresh: EventHandler<()>,
) -> Element {
    let mut show_create = use_signal(|| false);
    let mut new_user_id = use_signal(|| Option::<i32>::None);
    let mut new_description = use_signal(|| String::new());
    let mut creating = use_signal(|| false);
    let mut create_error = use_signal(|| Option::<String>::None);
    let mut created_token = use_signal(|| Option::<String>::None);

    let handle_create = move |_| {
        let user_id = *new_user_id.read();
        let desc = if new_description.read().is_empty() {
            None
        } else {
            Some(new_description.read().clone())
        };
        async move {
            creating.set(true);
            create_error.set(None);

            let server_url = SERVER_URL.read().clone();
            let token = TOKEN.read().clone();

            match create_token(&server_url, &token, user_id, desc.as_deref()).await {
                Ok(resp) => {
                    created_token.set(Some(resp.token));
                    new_user_id.set(None);
                    new_description.set(String::new());
                    on_refresh.call(());
                }
                Err(e) => create_error.set(Some(e)),
            }

            creating.set(false);
        }
    };

    rsx! {
        div {
            // Create section
            div { class: "mb-4",
                if let Some(token_value) = created_token.read().as_ref() {
                    div { class: "p-4 bg-green-50 rounded-lg border border-green-200 mb-4",
                        h3 { class: "font-medium text-green-800 mb-2", "Token Created!" }
                        p { class: "text-sm text-green-700 mb-2", "Copy this token now - it won't be shown again:" }
                        div { class: "flex gap-2",
                            code { class: "flex-1 p-2 bg-white rounded border border-green-200 text-sm font-mono break-all",
                                "{token_value}"
                            }
                        }
                        button {
                            class: "mt-3 px-4 py-2 text-sm bg-green-600 text-white rounded-lg hover:bg-green-700",
                            onclick: move |_| {
                                created_token.set(None);
                                show_create.set(false);
                            },
                            "Done"
                        }
                    }
                } else if *show_create.read() {
                    div { class: "p-4 bg-gray-50 rounded-lg border border-gray-200",
                        h3 { class: "font-medium text-gray-800 mb-3", "Create Token" }
                        if let Some(err) = create_error.read().as_ref() {
                            div { class: "mb-3 p-3 bg-red-50 text-red-700 text-sm rounded border border-red-200",
                                "{err}"
                            }
                        }
                        div { class: "space-y-3",
                            div {
                                label { class: "block text-sm text-gray-700 mb-1", "User (leave empty for system token)" }
                                select {
                                    class: "w-full px-3 py-2 border border-gray-200 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500",
                                    onchange: move |e| {
                                        let value = e.value();
                                        if value.is_empty() {
                                            new_user_id.set(None);
                                        } else if let Ok(id) = value.parse::<i32>() {
                                            new_user_id.set(Some(id));
                                        }
                                    },
                                    option { value: "", "System Token (Admin)" }
                                    for user in users.iter() {
                                        option { value: "{user.id}", "{user.name}" }
                                    }
                                }
                            }
                            div {
                                label { class: "block text-sm text-gray-700 mb-1", "Description (optional)" }
                                input {
                                    class: "w-full px-3 py-2 border border-gray-200 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500",
                                    r#type: "text",
                                    placeholder: "Token description",
                                    value: "{new_description}",
                                    oninput: move |e| new_description.set(e.value())
                                }
                            }
                            div { class: "flex gap-2",
                                button {
                                    class: "px-4 py-2 text-sm bg-blue-600 text-white rounded-lg hover:bg-blue-700 disabled:opacity-50",
                                    disabled: *creating.read(),
                                    onclick: handle_create,
                                    if *creating.read() { "Creating..." } else { "Create" }
                                }
                                button {
                                    class: "px-4 py-2 text-sm bg-gray-100 text-gray-700 rounded-lg hover:bg-gray-200",
                                    onclick: move |_| show_create.set(false),
                                    "Cancel"
                                }
                            }
                        }
                    }
                } else {
                    button {
                        class: "px-4 py-2 text-sm bg-blue-600 text-white rounded-lg hover:bg-blue-700",
                        onclick: move |_| show_create.set(true),
                        "+ Create Token"
                    }
                }
            }

            // Tokens table
            if tokens.is_empty() {
                div { class: "text-center py-8 text-gray-500",
                    "No tokens found"
                }
            } else {
                div { class: "overflow-x-auto",
                    table { class: "w-full text-sm",
                        thead { class: "bg-gray-50",
                            tr {
                                th { class: "px-4 py-3 text-left text-gray-600 font-medium", "ID" }
                                th { class: "px-4 py-3 text-left text-gray-600 font-medium", "Type" }
                                th { class: "px-4 py-3 text-left text-gray-600 font-medium", "User" }
                                th { class: "px-4 py-3 text-left text-gray-600 font-medium", "Description" }
                                th { class: "px-4 py-3 text-left text-gray-600 font-medium", "Created" }
                                th { class: "px-4 py-3 text-left text-gray-600 font-medium", "Actions" }
                            }
                        }
                        tbody {
                            for t in tokens.iter() {
                                TokenRow {
                                    key: "{t.id}",
                                    token_item: t.clone(),
                                    on_refresh: on_refresh.clone()
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
fn TokenRow(
    token_item: AdminTokenResponse,
    on_refresh: EventHandler<()>,
) -> Element {
    let mut deleting = use_signal(|| false);

    let token_id = token_item.id;

    let handle_delete = move |_| {
        async move {
            deleting.set(true);
            let server_url = SERVER_URL.read().clone();
            let auth_token = TOKEN.read().clone();
            if delete_token(&server_url, &auth_token, token_id).await.is_ok() {
                on_refresh.call(());
            }
            deleting.set(false);
        }
    };

    rsx! {
        tr { class: "border-t border-gray-100 hover:bg-gray-50",
            td { class: "px-4 py-3 text-gray-500 font-mono text-xs", "{token_item.id}" }
            td { class: "px-4 py-3",
                if token_item.is_system {
                    span { class: "px-2 py-1 text-xs font-medium bg-purple-100 text-purple-700 rounded", "System" }
                } else {
                    span { class: "px-2 py-1 text-xs font-medium bg-blue-100 text-blue-700 rounded", "User" }
                }
            }
            td { class: "px-4 py-3 text-gray-700",
                if let Some(name) = &token_item.user_name {
                    "{name}"
                } else {
                    "-"
                }
            }
            td { class: "px-4 py-3",
                InlineEdit {
                    value: token_item.description.clone(),
                    placeholder: "-",
                    on_save: move |new_desc: Option<String>| {
                        spawn(async move {
                            let server_url = SERVER_URL.read().clone();
                            let auth_token = TOKEN.read().clone();
                            if update_token(&server_url, &auth_token, token_id, new_desc.as_deref()).await.is_ok() {
                                on_refresh.call(());
                            }
                        });
                    }
                }
            }
            td { class: "px-4 py-3 text-gray-500 text-xs", "{token_item.created}" }
            td { class: "px-4 py-3",
                button {
                    class: "px-3 py-1 text-xs bg-red-50 text-red-600 rounded hover:bg-red-100 disabled:opacity-50",
                    disabled: *deleting.read(),
                    onclick: handle_delete,
                    "Delete"
                }
            }
        }
    }
}
