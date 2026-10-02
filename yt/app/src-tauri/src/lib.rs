mod args;
mod binaries;
mod commands;
mod disk;
mod errors;
mod jobs;
mod parser;
mod paths;
mod platform;
mod store;

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, RunEvent, WindowEvent};

/// Only hide the window on close if there is a tray icon to bring it back.
static TRAY_OK: AtomicBool = AtomicBool::new(false);

fn show_main(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn setup_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show YT Grab", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    let mut builder = TrayIconBuilder::new()
        .tooltip("YT Grab")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main(app),
            "quit" => {
                if let Some(st) = app.try_state::<jobs::Shared>() {
                    st.stop_all();
                }
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            let handle = app.handle().clone();
            let paths = paths::Paths::resolve(&handle);
            let state = jobs::AppState::new(std::sync::Arc::new(jobs::TauriHost(handle.clone())), paths);
            app.manage(state.clone());

            // A tray icon is optional: some Linux desktops have none.
            TRAY_OK.store(setup_tray(&handle).is_ok(), Ordering::Relaxed);

            // Resume anything that was queued or waiting when the app last closed.
            state.pump();

            // Subscription scheduler.
            let st = state.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    for id in st.due_subscriptions() {
                        st.check_subscription(&id);
                    }
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let keep_running = window
                    .app_handle()
                    .try_state::<jobs::Shared>()
                    .is_some_and(|st| st.settings().close_to_tray && st.has_active());
                if keep_running && TRAY_OK.load(Ordering::Relaxed) {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::install_tools,
            commands::latest_ytdlp_version,
            commands::get_settings,
            commands::save_settings,
            commands::probe,
            commands::preview_command,
            commands::list_jobs,
            commands::job_log,
            commands::add_job,
            commands::cancel_job,
            commands::retry_job,
            commands::remove_job,
            commands::clear_finished,
            commands::list_subscriptions,
            commands::save_subscription,
            commands::delete_subscription,
            commands::check_subscription_now,
            commands::reveal_path,
            commands::open_path,
            commands::quit_app,
        ])
        .build(tauri::generate_context!())
        .expect("error while building YT Grab");

    app.run(|handle, event| {
        if let RunEvent::Exit = event {
            if let Some(st) = handle.try_state::<jobs::Shared>() {
                st.stop_all();
            }
        }
    });
}
