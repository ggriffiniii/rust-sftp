#![feature(convert)]

extern crate sftp;
extern crate tempfile;

use std::convert::From;
use std::process;
use std::thread;
use std::io;
use std::io::Read;
use std::io::Write;

struct TempFile {
    file: tempfile::NamedTempFile,
    links: Vec<String>,
}

impl TempFile {
    fn new() -> TempFile {
        TempFile{file: tempfile::NamedTempFile::new().unwrap(), links: Vec::new()}
    }

    fn path(&self) -> String {
        From::from(self.file.path().to_str().unwrap())
    }

    fn symlink(&mut self) -> String {
        let link_path =
            tempfile::NamedTempFile::new().unwrap().path().to_str().unwrap().to_string();
        std::fs::soft_link(self.path(), &link_path).unwrap();
        self.links.push(link_path.clone());
        link_path
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        for path in self.links.iter() {
            std::fs::remove_file(path);
        }
    }
}

impl Read for TempFile {
    fn read(&mut self, buf: &mut[u8]) -> io::Result<usize> { self.file.read(buf) }
}

impl Write for TempFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> { self.file.write(buf) }
    fn flush(&mut self) -> io::Result<()> { self.file.flush() }
}

struct DebugWriter<W> {
    inner: W,
}           
            
impl<W : io::Write> io::Write for DebugWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        writeln!(&mut io::stderr(), "Writing to server: {:?}", buf).unwrap();
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
fn is_send() {
    let mut tempfile1 = TempFile::new();
    let mut tempfile2 = TempFile::new();
    tempfile1.write_all("file1".as_bytes()).unwrap();
    tempfile2.write_all("file2".as_bytes()).unwrap();
    let mut server = new_test_sftp_server().unwrap();
    //let r = DebugReader{inner: server.stdout.take().unwrap()};
    //let w = DebugWriter{inner: server.stdin.take().unwrap()};
    let r = server.stdout.take().unwrap();
    let w = server.stdin.take().unwrap();
    let mut client = sftp::Client::new(r, w).unwrap();
    let t1 = thread::spawn(move || {
        let mut file = client.open_options().read(true).open(tempfile1.path()).unwrap();
        let mut contents = String::new();
        file.read_to_string(&mut contents).unwrap();
        assert_eq!(contents.as_str(), "file1");
        let t2 = thread::spawn(move || {
            let mut file2 = client.open_options().read(true).open(tempfile2.path()).unwrap();
            let mut contents = String::new();
            file2.read_to_string(&mut contents).unwrap();
            assert_eq!(contents.as_str(), "file2");
            file.stat().unwrap()
        });
        t2.join()
    });
    t1.join().unwrap().unwrap();
    server.wait().unwrap();
}

#[test]
fn can_stat() {
    const contents : &'static str = "tempfile contents";
    let mut tempfile = TempFile::new();
    tempfile.write_all(contents.as_bytes()).unwrap();
    let mut server = new_test_sftp_server().unwrap();
    //let r = DebugReader{inner: server.stdout.take().unwrap()};
    //let w = DebugWriter{inner: server.stdin.take().unwrap()};
    let r = server.stdout.take().unwrap();
    let w = server.stdin.take().unwrap();
    {
        let mut client = sftp::Client::new(r, w).unwrap();
        let result = client.stat(tempfile.path()).unwrap();
        assert_eq!(contents.len() as u64, result.size.unwrap());
    }
    server.wait().unwrap();
}

#[test]
fn can_lstat() {
    const contents : &'static str = "tempfile contents";
    let mut tempfile = TempFile::new();
    tempfile.write_all(contents.as_bytes()).unwrap();
    let link = tempfile.symlink();
    let mut server = new_test_sftp_server().unwrap();
    //let r = DebugReader{inner: server.stdout.take().unwrap()};
    //let w = DebugWriter{inner: server.stdin.take().unwrap()};
    let r = server.stdout.take().unwrap();
    let w = server.stdin.take().unwrap();
    {
        let mut client = sftp::Client::new(r, w).unwrap();
        let lresult = client.lstat(link).unwrap();
        let result = client.stat(tempfile.path()).unwrap();
        assert!(lresult.size.unwrap() != result.size.unwrap());
    }
    server.wait().unwrap();
}

#[test]
fn can_read() {
    const contents : &'static str = "tempfile contents";
    let mut tempfile = TempFile::new();
    tempfile.write_all(contents.as_bytes()).unwrap();
    let mut server = new_test_sftp_server().unwrap();
    //let r = DebugReader{inner: server.stdout.take().unwrap()};
    //let w = DebugWriter{inner: server.stdin.take().unwrap()};
    let r = server.stdout.take().unwrap();
    let w = server.stdin.take().unwrap();
    {
        let mut client = sftp::Client::new(r, w).unwrap();
        let mut remote_file = client.open_options().read(true).open(tempfile.path()).unwrap();
        let mut remote_contents = vec![0,0,0,0];
        let n = remote_file.read(&mut remote_contents[..]).unwrap();
        assert_eq!(4, n);
        remote_file.read_to_end(&mut remote_contents).unwrap();
        assert_eq!(contents, String::from_utf8(remote_contents).unwrap());
    }
    server.wait().unwrap();
}

#[test]
fn can_write() {
    const contents : &'static str = "tempfile contents";
    let mut tempfile = TempFile::new();
    let mut server = new_test_sftp_server().unwrap();
    //let r = DebugReader{inner: server.stdout.take().unwrap()};
    //let w = DebugWriter{inner: server.stdin.take().unwrap()};
    let r = server.stdout.take().unwrap();
    let w = server.stdin.take().unwrap();
    {
        let mut client = sftp::Client::new(r, w).unwrap();
        let mut remote_file = client.open_options().write(true).open(tempfile.path()).unwrap();
        remote_file.write_all(contents.as_bytes()).unwrap();
        remote_file.write_all(contents.as_bytes()).unwrap();
    }
    server.wait().unwrap();
    let mut tempfile_contents = String::new();
    tempfile.read_to_string(&mut tempfile_contents).unwrap();
    let expected = {
        let mut x = String::from(contents);
        x.push_str(contents);
        x
    };
    assert_eq!(expected, tempfile_contents);
}

#[test]
fn can_remove() {
    let mut tempfile = TempFile::new();
    let mut server = new_test_sftp_server().unwrap();
    //let r = DebugReader{inner: server.stdout.take().unwrap()};
    //let w = DebugWriter{inner: server.stdin.take().unwrap()};
    let r = server.stdout.take().unwrap();
    let w = server.stdin.take().unwrap();
    {
        let mut client = sftp::Client::new(r, w).unwrap();
        // assert tempfile exists
        client.remove(tempfile.path()).unwrap();
        // assert tempfile no longer exists
    }
    server.wait().unwrap();
}
