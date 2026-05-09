mod commands;

use tauri::{
    Emitter, Manager,
    image::Image,
    menu::{MenuBuilder, MenuItemBuilder, PredefinedMenuItem},
    tray::TrayIconBuilder,
};

fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let show = MenuItemBuilder::with_id("show", "Show Window").build(app)?;
    let hide = MenuItemBuilder::with_id("hide", "Hide Window").build(app)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sessions = MenuItemBuilder::with_id("sessions", "Sessions").build(app)?;
    let memory = MenuItemBuilder::with_id("memory", "Memory").build(app)?;
    let dashboard = MenuItemBuilder::with_id("dashboard", "Dashboard").build(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;

    let menu = MenuBuilder::new(app)
        .items(&[
            &show, &hide, &sep1, &dashboard, &sessions, &memory, &sep2, &quit,
        ])
        .build()?;

    let tray_icon = Image::from_path("icons/32x32.png")
        .or_else(|_| Image::from_bytes(include_bytes!("../icons/32x32.png")))
        .unwrap_or_else(|_| Image::from_bytes(include_bytes!("../icons/icon.png")).unwrap());

    TrayIconBuilder::new()
        .icon(tray_icon)
        .tooltip("Asterel")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            }
            "hide" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }
            "sessions" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
                let _ = app.emit("tray-navigate", "/sessions");
            }
            "dashboard" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
                let _ = app.emit("tray-navigate", "/dashboard");
            }
            "memory" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
                let _ = app.emit("tray-navigate", "/memory");
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let tauri::tray::TrayIconEvent::Click {
                button: tauri::tray::MouseButton::Left,
                button_state: tauri::tray::MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(window) = app.get_webview_window("main") {
                    if window.is_visible().unwrap_or(false) {
                        let _ = window.hide();
                    } else {
                        let _ = window.show();
                        let _ = window.unminimize();
                        let _ = window.set_focus();
                    }
                }
            }
        })
        .build(app)?;

    Ok(())
}

fn setup_deep_links(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(desktop)]
    {
        use tauri_plugin_deep_link::DeepLinkExt;
        // Forward any URLs received at launch to the frontend via event.
        if let Ok(Some(urls)) = app.deep_link().get_current() {
            let payload: Vec<String> = urls.iter().map(|u| u.to_string()).collect();
            let handle = app.handle().clone();
            // Defer the emit so the webview is ready.
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                let _ = handle.emit("deep-link-received", payload);
            });
        }
        let handle = app.handle().clone();
        app.deep_link().on_open_url(move |event| {
            let payload: Vec<String> = event.urls().iter().map(|u| u.to_string()).collect();
            let _ = handle.emit("deep-link-received", payload);
            if let Some(window) = handle.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        });
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // WebKitGTK DMA-BUF rendering can fail on many GPU drivers under Wayland.
    // Keep the generic workaround for Intel/AMD, plus NVIDIA-specific fixes.
    #[cfg(target_os = "linux")]
    {
        // if std::env::var("WEBKIT_DISABLE_DMABUF_RENDERER").is_err() {
        //     unsafe { std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1") };
        // }
        // Hyprland + NVIDIA: explicit sync causes protocol error 71.
        if std::env::var("__NV_DISABLE_EXPLICIT_SYNC").is_err() {
            unsafe { std::env::set_var("__NV_DISABLE_EXPLICIT_SYNC", "1") };
        }
        if std::env::var("GSK_RENDERER").is_err() {
            unsafe { std::env::set_var("GSK_RENDERER", "ngl") };
        }
    }
    tauri::Builder::default()
        // Single-instance MUST be registered first — it may exit the process.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_log::Builder::new().build())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_websocket::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_deep_link::init())
        .setup(|app| {
            setup_tray(app)?;
            setup_deep_links(app)?;
            #[cfg(debug_assertions)]
            if let Some(window) = app.get_webview_window("main") {
                window.open_devtools();
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::daemon_request,
            commands::health_check,
            commands::pair_with_daemon,
            commands::send_notification,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
