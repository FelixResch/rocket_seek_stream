#![feature(proc_macro_hygiene, decl_macro)]

/// Run download-videos.sh to download the videos required to run this example
/// It depends on youtube-dl.
/// 
/// Use `cargo run --example server` then navigate to [localhost:8000](http://localhost:8000)
/// in your browser.
/// 
/// Change the path between "/"", "from_path", and "long" to see the different examples in action.

#[macro_use]
extern crate rocket;

use rocket::http::ContentType;
use rocket_seek_stream::SeekStream;

// stream from an in memory buffer
#[get("/memory")]
fn hello() -> SeekStream {
    let bytes = &include_bytes!("./fly_me_to_the_moon.webm")[..];
    let len = bytes.len();
    let stream = std::io::Cursor::new(bytes);

    SeekStream::with_opts(stream, len as u64, "video/webm")
}

// stream from a given filepath
#[get("/from_path")]
async fn from_path() -> std::io::Result<SeekStream> {
    SeekStream::from_path("examples/fly_me_to_the_moon.webm").await
}

// some long media
#[get("/long")]
async fn long() -> std::io::Result<SeekStream> {
    SeekStream::from_path("fly_me_to_the_moon.mkv").await
}

// some longer media
#[get("/")]
fn longer() -> (ContentType, &'static str) {
    (
        ContentType::HTML,
        r#"<!DOCTYPE html>
        <html>
        <head>
        </head>
        <body>
            <video src="/from_path" controls />
        </body>
        </html>"#
        )
}

#[rocket::launch]
fn launch() -> _ {
    rocket::build()
        .mount("/", routes![hello, from_path, long, longer])
}
