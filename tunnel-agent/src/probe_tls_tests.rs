use std::{
    net::{TcpListener, TcpStream},
    process::{Child, Command, Stdio},
};

use super::*;

struct Fixture {
    dir: PathBuf,
    server: Option<Child>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(server) = &mut self.server {
            let _ = server.kill();
            let _ = server.wait();
        }
        let _ = fs::remove_dir_all(&self.dir);
    }
}

impl Fixture {
    fn openssl(&self, args: &[&str]) -> Command {
        let mut command = Command::new("openssl");
        command
            .args(args)
            .current_dir(&self.dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }

    fn start(port: u16) -> Self {
        let dir = std::env::temp_dir().join(crate::release::unique_suffix().unwrap());
        fs::create_dir(&dir).unwrap();
        let mut fixture = Fixture { dir, server: None };
        fs::write(
            fixture.dir.join("sample"),
            vec![42; SAMPLE_BYTES as usize * 2],
        )
        .unwrap();
        assert!(
            fixture
                .openssl(&[
                    "req",
                    "-x509",
                    "-nodes",
                    "-newkey",
                    "ec",
                    "-pkeyopt",
                    "ec_paramgen_curve:prime256v1",
                    "-subj",
                    "/CN=fixture.example",
                    "-addext",
                    "subjectAltName=DNS:fixture.example",
                    "-addext",
                    "basicConstraints=critical,CA:FALSE",
                    "-days",
                    "1",
                    "-keyout",
                    "key.pem",
                    "-out",
                    "cert.pem",
                ])
                .status()
                .unwrap()
                .success()
        );
        fixture.server = Some(
            fixture
                .openssl(&[
                    "s_server",
                    "-quiet",
                    "-WWW",
                    "-cert",
                    "cert.pem",
                    "-key",
                    "key.pem",
                    "-accept",
                    &format!("127.0.0.1:{port}"),
                ])
                .spawn()
                .unwrap(),
        );
        for _ in 0..200 {
            assert!(
                fixture
                    .server
                    .as_mut()
                    .unwrap()
                    .try_wait()
                    .unwrap()
                    .is_none()
            );
            if TcpStream::connect((Ipv4Addr::LOCALHOST, port)).is_ok() {
                return fixture;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("TLS fixture failed to listen");
    }
}

#[test]
fn verifies_tls_and_bounds_a_real_download() {
    let socket = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = socket.local_addr().unwrap().port();
    drop(socket);
    let fixture = Fixture::start(port);
    let mut job = Job {
        id: "930fad72-e7d4-4311-8711-591bfaf5b64f".to_owned(),
        starts_at: 100,
        ends_at: 700,
        port,
        hostname: "fixture.example".to_owned(),
        path: "/sample".to_owned(),
        expected_status: 200,
        download: true,
    };
    job.validate().unwrap();
    assert_eq!(
        (job.slot(99), job.slot(100), job.slot(699), job.slot(700)),
        (None, Some(1), Some(20), None)
    );
    let url = format!("https://{}:{port}/sample", job.hostname);
    let untrusted = probe_client(&job).build().unwrap();
    assert!(matches!(
        measure(&untrusted, &job, &url),
        Sample::Failed(Failure::Transport)
    ));
    let cert = fs::read(fixture.dir.join("cert.pem")).unwrap();
    let client = probe_client(&job)
        .add_root_certificate(reqwest::Certificate::from_pem(&cert).unwrap())
        .build()
        .unwrap();
    assert!(matches!(
        measure(&client, &job, &url),
        Sample::Passed {
            bytes: SAMPLE_BYTES,
            ..
        }
    ));
    job.expected_status = 206;
    assert!(matches!(
        measure(&client, &job, &url),
        Sample::Failed(Failure::Status)
    ));
}
