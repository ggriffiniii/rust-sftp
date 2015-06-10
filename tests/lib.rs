extern crate sftp;

use std::process;
use std::io;
use std::io::Write;

struct DebugWriter<W> {
    inner: W,
}           
            
impl<W : io::Write> io::Write for DebugWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        writeln!(&mut io::stderr(), "Writing to server: {:?}", buf);
        self.inner.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
                   
struct DebugReader<R> {
    inner: R,
}

impl<R : io::Read> io::Read for DebugReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let res = self.inner.read(buf);
        writeln!(&mut io::stderr(), "Reading from server: {:?}", buf);
        res
    }
}

fn new_test_sftp_server() -> io::Result<process::Child> {
    let sftp_cmd = if cfg!(target_os = "macos") {
        "/usr/libexec/sftp-server"
    } else {
        "/usr/lib/openssh/sftp-server"
    };
	process::Command::new(sftp_cmd)
        .arg("-e")
		.stdin(process::Stdio::piped())
		.stdout(process::Stdio::piped())
		.stderr(process::Stdio::inherit())
		.spawn()
}

#[test]
fn it_works() {
	let mut server = new_test_sftp_server().unwrap();
	{
        let mut r = DebugReader{inner: server.stdout.take().unwrap()};
        let mut w = DebugWriter{inner: server.stdin.take().unwrap()};
        //let mut r = server.stdout.take().unwrap();
        //let mut w = server.stdin.take().unwrap();
		let mut client = sftp::Client::new(r, w).unwrap();
        let attr = client.stat("/".to_string()).unwrap();
        let size = attr.size.unwrap();
        let file = client.open("/foo".to_string());
        assert!(size == 4096);
        
	}
	server.wait().unwrap();
}
