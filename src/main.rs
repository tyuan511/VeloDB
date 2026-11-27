use gpui::{
    px, size, App, AppContext, Bounds, TitlebarOptions, WindowBackgroundAppearance,
    WindowBounds, WindowOptions,
};
use gpui_component::Root;
use velodb::ui::app::VeloDbApp;

fn main() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to create Tokio runtime");

    let rt_handle = rt.handle().clone();

    gpui_platform::application().run(move |cx: &mut App| {
        gpui_component::init(cx);

        let bounds = Bounds::centered(None, size(px(1440.0), px(900.0)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("VeloDB".into()),
                appears_transparent: true,
                traffic_light_position: Some(gpui::point(px(12.0), px(14.0))),
            }),
            app_owns_titlebar_drag: true,
            window_background: WindowBackgroundAppearance::Blurred,
            focus: true,
            show: true,
            ..Default::default()
        };

        cx.open_window(options, move |window, cx| {
            let app_view = cx.new(|cx| VeloDbApp::new(rt_handle.clone(), window, cx));
            cx.new(|cx| Root::new(app_view, window, cx))
        })
        .expect("Failed to open VeloDB main window");

        cx.activate(true);
    });
}
