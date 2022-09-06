use axum::response::Redirect;
use axum::routing::{get, get_service};
use axum::Router;

mod fs;
mod thumbnail;

pub fn configure() -> Router {
	let mut router = Router::new();

	router = router.route("/", get(|| async { Redirect::permanent("/fs/") }));
	router = router.nest("/thumb", thumbnail::configure());
	router = router.nest("/fs", fs::configure());
	router = router.nest("/static", get_service(static_router())); // work around axum special-casing nesting `Router`s

	router
}

static_router::static_router!(static_router, "static");
