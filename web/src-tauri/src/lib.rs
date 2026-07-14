mod monitor;

use monitor::Monitor;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  let monitor = Monitor::new();
  let monitor_for_factory = monitor.clone();
  let monitor_for_setup = monitor.clone();

  tauri::Builder::default()
    .plugin(tauri_plugin_notification::init())
    .plugin(tauri_plugin_background_service::init_with_service(move || {
      monitor::service::KilnMonitorService::new(monitor_for_factory.clone())
    }))
    .manage(monitor)
    .invoke_handler(tauri::generate_handler![
      monitor::commands::set_kiln_url,
      monitor::commands::get_kiln_status,
      monitor::commands::get_kiln_history,
      monitor::commands::monitoring_status,
      monitor::commands::refresh_kiln,
    ])
    .setup(move |app| {
      if cfg!(debug_assertions) {
        app.handle().plugin(
          tauri_plugin_log::Builder::default()
            .level(log::LevelFilter::Info)
            .build(),
        )?;
      }
      monitor_for_setup.attach(app.handle());
      monitor_for_setup.spawn_supervisor(app.handle().clone());
      Ok(())
    })
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}
