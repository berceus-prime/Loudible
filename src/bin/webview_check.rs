//! Checks the embedded sign-in browser inside the real sheet.
//!
//! By default it talks only to a loopback server: the browser must present the user agent and
//! cookies it was given, and a server redirect to the callback must be intercepted before it
//! loads. `--amazon` instead opens Amazon's sign-in page for a freshly generated device, for a
//! person to try by hand; the authorization code is never printed or exchanged.

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn main() {
    native::run();
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn main() {
    eprintln!("This platform has no embedded browser yet.");
    std::process::exit(2);
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod native {
    use std::{
        io::{BufRead, BufReader, Write},
        net::{TcpListener, TcpStream},
        process::exit,
        sync::{Arc, Mutex},
        thread,
        time::Duration,
    };

    use loudible_adapters::{AudibleLoginFactory, SystemRandomSource};
    use loudible_core::{Marketplace, SystemClock};
    use skia_ui::{
        model::MockOptions,
        shell::{
            app_view::AppView,
            browser::{BrowserCookie, BrowserPage},
            window,
        },
    };

    const USER_AGENT: &str = "LoudibleWebviewCheck/1.0 (iPhone)";
    const COOKIES: [&str; 3] = ["frc", "map-md", "amzn-app-id"];
    const TIME_LIMIT: Duration = Duration::from_secs(30);
    const WINDOW: (f64, f64) = (1100.0, 800.0);

    #[derive(Default)]
    struct Seen {
        user_agent: String,
        cookie_header: String,
        reached_callback: bool,
    }

    pub fn run() {
        if std::env::args().any(|arg| arg == "--amazon") {
            amazon();
        } else {
            loopback();
        }
    }

    fn start(page: BrowserPage, on_callback: impl FnMut(&str) + 'static) {
        let mut on_callback = on_callback;
        let mut view = AppView::new(MockOptions::default()).with_browser(
            page,
            Box::new(move |url| {
                on_callback(url);
                None
            }),
        );
        view.app.sign_in();
        window::run(view, "Loudible webview check", WINDOW);
    }

    fn amazon() {
        let factory = AudibleLoginFactory::new(Arc::new(SystemRandomSource), Arc::new(SystemClock));
        let page = BrowserPage::for_login(&factory.make_login(Marketplace::US));
        start(page, |url| {
            let has_code = url.contains("openid.oa2.authorization_code=");
            println!("Intercepted the callback. Authorization code present: {has_code}");
            exit(0);
        });
    }

    fn loopback() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let address = listener.local_addr().expect("local address");
        let seen = Arc::new(Mutex::new(Seen::default()));
        let server_seen = seen.clone();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                serve(stream, &server_seen);
            }
        });
        thread::spawn(|| {
            thread::sleep(TIME_LIMIT);
            eprintln!("FAIL  no callback within {TIME_LIMIT:?}");
            exit(1);
        });

        let host = address.ip().to_string();
        let cookies = COOKIES
            .map(|name| BrowserCookie {
                name: name.into(),
                value: format!("value-of-{name}"),
                domain: host.clone(),
                // Loopback is plain HTTP, which a secure cookie would never reach.
                is_secure: false,
            })
            .to_vec();
        let page = BrowserPage {
            url: format!("http://{address}/ap/signin"),
            user_agent: USER_AGENT.into(),
            cookies,
            callback_host: host,
            callback_path: "/ap/maplanding".into(),
        };
        start(page, move |url| {
            let seen = seen.lock().expect("lock");
            let checks = [
                ("user agent presented", seen.user_agent == USER_AGENT),
                (
                    "cookies sent",
                    COOKIES.iter().all(|name| {
                        seen.cookie_header
                            .contains(&format!("{name}=value-of-{name}"))
                    }),
                ),
                (
                    "redirect delivered",
                    url.contains("authorization_code=loopback"),
                ),
                ("callback never loaded", !seen.reached_callback),
            ];
            for (name, passed) in checks {
                println!("{}  {name}", if passed { "ok  " } else { "FAIL" });
            }
            exit(if checks.iter().all(|(_, passed)| *passed) {
                0
            } else {
                1
            });
        });
    }

    fn serve(mut stream: TcpStream, seen: &Mutex<Seen>) {
        let mut lines = BufReader::new(stream.try_clone().expect("clone stream")).lines();
        let Some(Ok(request)) = lines.next() else {
            return;
        };
        let path = request.split(' ').nth(1).unwrap_or("/").to_string();
        let mut user_agent = String::new();
        let mut cookie_header = String::new();
        for line in lines
            .map_while(Result::ok)
            .take_while(|line| !line.is_empty())
        {
            if let Some((name, value)) = line.split_once(':') {
                match name.to_ascii_lowercase().as_str() {
                    "user-agent" => user_agent = value.trim().to_string(),
                    "cookie" => cookie_header = value.trim().to_string(),
                    _ => {}
                }
            }
        }
        let response = if path == "/ap/signin" {
            let mut seen = seen.lock().expect("lock");
            seen.user_agent = user_agent;
            seen.cookie_header = cookie_header;
            let body = "<!doctype html><body style='font:20px sans-serif;background:#cde'>\
                <h1>Webview check</h1>\
                <script>setTimeout(function(){location.href='/redirect'},1500)</script>";
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
        } else if path == "/redirect" {
            "HTTP/1.1 302 Found\r\nLocation: /ap/maplanding?openid.oa2.authorization_code=loopback\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string()
        } else {
            if path.starts_with("/ap/maplanding") {
                seen.lock().expect("lock").reached_callback = true;
            }
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string()
        };
        let _ = stream.write_all(response.as_bytes());
    }
}
