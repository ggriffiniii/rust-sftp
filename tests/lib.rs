#![feature(convert)]

extern crate sftp;
extern crate tempfile;
extern crate tempdir;

use std::collections::HashMap;
use std::convert::From;
use std::process;
use std::thread;
use std::io;
use std::io::Read;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::fs::File;

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
            let _ = std::fs::remove_file(path);
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
        writeln!(&mut io::stderr(), "Reading from server: {:?}", buf).unwrap();
        res
    }
}

struct TestSftpServer {
    server: process::Child,
}

impl TestSftpServer {
    fn new() -> TestSftpServer {
        let sftp_cmd = if cfg!(target_os = "macos") {
            "/usr/libexec/sftp-server"
        } else {
            "/usr/lib/openssh/sftp-server"
        };
        let server = process::Command::new(sftp_cmd)
            .arg("-e")
            //.arg("-l")
            //.arg("DEBUG")
            .stdin(process::Stdio::piped())
            .stdout(process::Stdio::piped())
            .stderr(process::Stdio::inherit())
            .spawn()
            .unwrap();
        TestSftpServer{server: server}
    }

    fn client(&mut self) -> sftp::Client<process::ChildStdin> {
        let r = self.server.stdout.take().unwrap();
        let w = self.server.stdin.take().unwrap();
        sftp::Client::new(r, w).unwrap()
    }

    #[allow(dead_code)]
    fn debug_client(&mut self) -> sftp::Client<DebugWriter<process::ChildStdin>> {
        let r = DebugReader{inner: self.server.stdout.take().unwrap()};
        let w = DebugWriter{inner: self.server.stdin.take().unwrap()};
        sftp::Client::new(r, w).unwrap()
    }
}

impl Drop for TestSftpServer {
    fn drop(&mut self) {
        self.server.wait().unwrap();
    }
}

#[test]
fn is_send() {
    let mut tempfile1 = TempFile::new();
    let mut tempfile2 = TempFile::new();
    tempfile1.write_all("file1".as_bytes()).unwrap();
    tempfile2.write_all("file2".as_bytes()).unwrap();
    let mut server = TestSftpServer::new();
    let mut client = server.client();
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
}

#[test]
fn can_stat() {
    const CONTENTS : &'static str = "tempfile contents";
    let mut tempfile = TempFile::new();
    tempfile.write_all(CONTENTS.as_bytes()).unwrap();
    let mut server = TestSftpServer::new();
    let mut client = server.client();
    let result = client.stat(tempfile.path()).unwrap();
    assert_eq!(CONTENTS.len() as u64, result.size.unwrap());
}

#[test]
fn can_lstat() {
    const CONTENTS : &'static str = "tempfile contents";
    let mut tempfile = TempFile::new();
    tempfile.write_all(CONTENTS.as_bytes()).unwrap();
    let link = tempfile.symlink();
    let mut server = TestSftpServer::new();
    let mut client = server.client();
    let lresult = client.lstat(link).unwrap();
    let result = client.stat(tempfile.path()).unwrap();
    assert!(lresult.size.unwrap() != result.size.unwrap());
}

#[test]
fn can_read() {
    const CONTENTS : &'static str = "tempfile contents";
    let mut tempfile = TempFile::new();
    tempfile.write_all(CONTENTS.as_bytes()).unwrap();
    let mut server = TestSftpServer::new();
    let mut client = server.client();
    let mut remote_file = client.open_options().read(true).open(tempfile.path()).unwrap();
    let mut remote_contents = vec![0,0,0,0];
    let n = remote_file.read(&mut remote_contents[..]).unwrap();
    assert_eq!(4, n);
    remote_file.read_to_end(&mut remote_contents).unwrap();
    assert_eq!(CONTENTS, String::from_utf8(remote_contents).unwrap());
}

#[test]
fn can_write() {
    const CONTENTS : &'static str = "tempfile contents";
    let mut tempfile = TempFile::new();
    let mut server = TestSftpServer::new();
    let mut client = server.client();
    let mut remote_file = client.open_options().write(true).open(tempfile.path()).unwrap();
    remote_file.write_all(CONTENTS.as_bytes()).unwrap();
    remote_file.write_all(CONTENTS.as_bytes()).unwrap();
    let mut tempfile_contents = String::new();
    tempfile.read_to_string(&mut tempfile_contents).unwrap();
    let expected = {
        let mut x = String::from(CONTENTS);
        x.push_str(CONTENTS);
        x
    };
    assert_eq!(expected, tempfile_contents);
}

#[test]
fn can_remove() {
    let tempfile = TempFile::new();
    let mut server = TestSftpServer::new();
    let mut client = server.client();
    match std::fs::metadata(tempfile.path()) {
        Ok(_) => {},
        Err(_) => panic!("file doesn't exist"),
    }
    client.remove(tempfile.path()).unwrap();
    match std::fs::metadata(tempfile.path()) {
        Ok(_) => panic!("file still exists"),
        Err(_) => {},
    }
}

#[test]
fn can_mkdir() {
    let tempfile_path = TempFile::new().path();
    let mut server = TestSftpServer::new();
    let mut client = server.client();
    match std::fs::metadata(&tempfile_path) {
        Ok(_) => panic!("file already exists: {}", &tempfile_path),
        _ => {},
    }
    client.mkdir(tempfile_path.clone()).unwrap();
    match std::fs::metadata(&tempfile_path) {
        Ok(metadata) => assert!(metadata.is_dir()),
        Err(x) => panic!("unable to stat directory {}: {:?}", &tempfile_path, x),
    }
    std::fs::remove_dir(&tempfile_path).unwrap();
}

#[test]
fn can_rmdir() {
    let tempfile_path = TempFile::new().path();
    let mut server = TestSftpServer::new();
    let mut client = server.client();
    std::fs::create_dir(&tempfile_path).unwrap();
    client.rmdir(tempfile_path.clone()).unwrap();
    match std::fs::metadata(&tempfile_path) {
        Ok(_) => panic!("directory still exists: {}", tempfile_path),
        Err(_) => {},
    }
}

#[test]
fn can_setstat() {
    let tempfile = TempFile::new();
    let mut server = TestSftpServer::new();
    let mut client = server.client();
    let mut new_attrs = sftp::FileAttr::new();
    new_attrs.mtime = Some(0);
    new_attrs.atime = Some(0);
    client.setstat(tempfile.path(), new_attrs).unwrap();
    let mtime = std::fs::metadata(tempfile.path()).unwrap().mtime();
    assert_eq!(0, mtime);
}

#[test]
fn can_fsetstat() {
    let tempfile = TempFile::new();
    let mut server = TestSftpServer::new();
    let mut client = server.client();
    let mut new_attrs = sftp::FileAttr::new();
    new_attrs.mtime = Some(0);
    new_attrs.atime = Some(0);
    let mut file = client.open_options().read(true).open(tempfile.path()).unwrap();
    file.setstat(new_attrs).unwrap();
    let mtime = std::fs::metadata(tempfile.path()).unwrap().mtime();
    assert_eq!(0, mtime);
}

#[test]
fn can_realpath() {
    let tempfile = TempFile::new();
    let mut server = TestSftpServer::new();
    let mut client = server.client();
    let name = client.realpath(tempfile.path()).unwrap();
    assert_eq!(tempfile.path().into_bytes(), name.filename);
}

#[test]
fn can_rename() {
    let tempfile = TempFile::new();
    let newpath = TempFile::new().path();
    let mut server = TestSftpServer::new();
    let mut client = server.client();
    match std::fs::metadata(&newpath) {
        Ok(_) => panic!("file already exists: {}", &newpath),
        _ => {},
    }
    client.rename(tempfile.path(), newpath.clone()).unwrap();
    std::fs::metadata(&newpath).unwrap();
}

#[test]
fn can_readlink() {
    const LINK_TARGET : &'static str = "/tmp/foobar";
    let linkpath = TempFile::new().path();
    std::os::unix::fs::symlink(LINK_TARGET, &linkpath).unwrap();
    let mut server = TestSftpServer::new();
    let mut client = server.client();
    let dst = client.readlink(linkpath.clone()).unwrap();
    assert_eq!(LINK_TARGET.as_bytes(), dst.filename.as_slice());
    let _ = std::fs::remove_file(linkpath);
}

#[test]
fn can_readdir() {
    let tmp_dir = tempdir::TempDir::new("sftp_readdir").unwrap();
    let mut files = HashMap::new();
    let mut path = tmp_dir.path().to_path_buf();
    path.push("filename");
    for i in 0..100 {
        let fname = format!("file-{}", i);
        files.insert(fname.to_string(), ());
        path.set_file_name(fname);
        File::create(&path).unwrap();
    }
    let mut server = TestSftpServer::new();
    let mut client = server.client();
    for file in client.readdir(tmp_dir.path().to_str().unwrap().to_string()).unwrap().map(|x| x.unwrap()) {
        let fname = String::from_utf8(file.filename).unwrap();
        if fname == "." || fname == ".." {
            continue;
        }
        match files.remove(&fname) {
            None => panic!("unexpected file returned from readdir: {:?}", fname),
            Some(_) => {},
        }
    }
    assert_eq!(0, files.len());
}
