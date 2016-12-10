use std::ffi::CStr;
use std::fmt;
use std::io;
use std::io::prelude::*;
use std::marker::PhantomData;
use std::mem;
#[cfg(unix)]
use std::os::unix::io::AsRawFd;
use std::ptr;
use std::result;
use std::slice;
use std::str::Utf8Error;

use libc;
use ffi;

use IntoNativeString;
use error::{self, Error, Result};

ffi_enum_wrapper! {
    pub enum Encoding: ffi::gpgme_data_encoding_t {
        ENCODING_NONE = ffi::GPGME_DATA_ENCODING_NONE,
        ENCODING_BINARY = ffi::GPGME_DATA_ENCODING_BINARY,
        ENCODING_BASE64 = ffi::GPGME_DATA_ENCODING_BASE64,
        ENCODING_ARMOR = ffi::GPGME_DATA_ENCODING_ARMOR,
        ENCODING_URL = ffi::GPGME_DATA_ENCODING_URL,
        ENCODING_URLESC = ffi::GPGME_DATA_ENCODING_URLESC,
        ENCODING_URL0 = ffi::GPGME_DATA_ENCODING_URL0,
        ENCODING_MIME = ffi::GPGME_DATA_ENCODING_MIME,
    }
}

ffi_enum_wrapper! {
    pub enum Type: ffi::gpgme_data_type_t {
        TYPE_UNKNOWN = ffi::GPGME_DATA_TYPE_UNKNOWN,
        TYPE_INVALID = ffi::GPGME_DATA_TYPE_INVALID,
        TYPE_PGP_SIGNED = ffi::GPGME_DATA_TYPE_PGP_SIGNED,
        TYPE_PGP_ENCRYPTED = ffi::GPGME_DATA_TYPE_PGP_ENCRYPTED,
        TYPE_PGP_OTHER = ffi::GPGME_DATA_TYPE_PGP_OTHER,
        TYPE_PGP_KEY = ffi::GPGME_DATA_TYPE_PGP_KEY,
        TYPE_PGP_SIGNATURE = ffi::GPGME_DATA_TYPE_PGP_SIGNATURE,
        TYPE_CMS_SIGNED = ffi::GPGME_DATA_TYPE_CMS_SIGNED,
        TYPE_CMS_ENCRYPTED = ffi::GPGME_DATA_TYPE_CMS_ENCRYPTED,
        TYPE_CMS_OTHER = ffi::GPGME_DATA_TYPE_CMS_OTHER,
        TYPE_X509_CERT = ffi::GPGME_DATA_TYPE_X509_CERT,
        TYPE_PKCS12 = ffi::GPGME_DATA_TYPE_PKCS12,
    }
}

struct CallbackWrapper<S> {
    cbs: ffi::gpgme_data_cbs,
    inner: S,
}

#[derive(Clone)]
pub struct WrappedError<S>(Error, S);

impl<S> WrappedError<S> {
    pub fn error(&self) -> Error {
        self.0
    }

    pub fn into_inner(self) -> S {
        self.1
    }
}

impl<S> fmt::Debug for WrappedError<S> {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&self.0, fmt)
    }
}

impl<S> fmt::Display for WrappedError<S> {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&self.0, fmt)
    }
}

#[derive(Debug)]
pub struct Data<'a> {
    raw: ffi::gpgme_data_t,
    phantom: PhantomData<&'a ()>,
}

impl<'a> Data<'a> {
    pub unsafe fn from_raw(raw: ffi::gpgme_data_t) -> Self {
        debug_assert!(!raw.is_null());
        Data {
            raw: raw,
            phantom:  PhantomData,
        }
    }

    pub fn as_raw(&self) -> ffi::gpgme_data_t {
        self.raw
    }

    pub fn into_raw(self) -> ffi::gpgme_data_t {
        let raw = self.raw;
        mem::forget(self);
        raw
    }

    pub fn stdin() -> Result<Data<'static>> {
        Data::from_reader(io::stdin()).map_err(|err| err.error())
    }

    pub fn stdout() -> Result<Data<'static>> {
        Data::from_writer(io::stdout()).map_err(|err| err.error())
    }

    pub fn stderr() -> Result<Data<'static>> {
        Data::from_writer(io::stderr()).map_err(|err| err.error())
    }

    /// Constructs an empty data object.
    pub fn new() -> Result<Data<'static>> {
        let mut data = ptr::null_mut();
        unsafe {
            return_err!(ffi::gpgme_data_new(&mut data));
            Ok(Data::from_raw(data))
        }
    }

    /// Constructs a data object and fills it with the contents of the file
    /// referenced by `path`.
    pub fn load<P: IntoNativeString>(path: P) -> Result<Data<'static>> {
        let path = path.into_native();
        unsafe {
            let mut data = ptr::null_mut();
            return_err!(ffi::gpgme_data_new_from_file(&mut data, path.as_ref().as_ptr(), 1));
            Ok(Data::from_raw(data))
        }
    }

    /// Constructs a data object and fills it with a copy of `bytes`.
    pub fn from_bytes<B: AsRef<[u8]>>(bytes: B) -> Result<Data<'static>> {
        let bytes = bytes.as_ref();
        unsafe {
            let (buf, len) = (bytes.as_ptr() as *const _, bytes.len().into());
            let mut data = ptr::null_mut();
            return_err!(ffi::gpgme_data_new_from_mem(&mut data, buf, len, 1));
            Ok(Data::from_raw(data))
        }
    }

    /// Constructs a data object which copies from `buf` as needed.
    pub fn from_buffer<B: AsRef<[u8]> + ?Sized>(buf: &B) -> Result<Data> {
        let buf = buf.as_ref();
        unsafe {
            let (buf, len) = (buf.as_ptr() as *const _, buf.len().into());
            let mut data = ptr::null_mut();
            return_err!(ffi::gpgme_data_new_from_mem(&mut data, buf, len, 0));
            Ok(Data::from_raw(data))
        }
    }

    #[cfg(unix)]
    pub fn from_fd<T: AsRawFd + ?Sized>(file: &T) -> Result<Data> {
        unsafe {
            let mut data = ptr::null_mut();
            return_err!(ffi::gpgme_data_new_from_fd(&mut data, file.as_raw_fd()));
            Ok(Data::from_raw(data))
        }
    }

    pub unsafe fn from_raw_file<'b>(file: *mut libc::FILE) -> Result<Data<'b>> {
        let mut data = ptr::null_mut();
        return_err!(ffi::gpgme_data_new_from_stream(&mut data, file));
        Ok(Data::from_raw(data))
    }

    unsafe fn from_callbacks<S>(cbs: ffi::gpgme_data_cbs, src: S)
        -> result::Result<Data<'static>, WrappedError<S>>
    where S: Send + 'static {
        let src = Box::into_raw(Box::new(CallbackWrapper {
            cbs: cbs,
            inner: src,
        }));
        let cbs = &mut (*src).cbs as *mut _;
        let mut data = ptr::null_mut();
        let result = ffi::gpgme_data_new_from_cbs(&mut data, cbs, src as *mut _);
        if result == 0 {
            Ok(Data::from_raw(data))
        } else {
            Err(WrappedError(Error::new(result), Box::from_raw(src).inner))
        }
    }

    pub fn from_reader<R>(r: R) -> result::Result<Data<'static>, WrappedError<R>>
    where R: Read + Send + 'static {
        let cbs = ffi::gpgme_data_cbs {
            read: Some(read_callback::<R>),
            write: None,
            seek: None,
            release: Some(release_callback::<R>),
        };
        unsafe { Data::from_callbacks(cbs, r) }
    }

    pub fn from_seekable_reader<R>(r: R) -> result::Result<Data<'static>, WrappedError<R>>
    where R: Read + Seek + Send + 'static {
        let cbs = ffi::gpgme_data_cbs {
            read: Some(read_callback::<R>),
            write: None,
            seek: Some(seek_callback::<R>),
            release: Some(release_callback::<R>),
        };
        unsafe { Data::from_callbacks(cbs, r) }
    }

    pub fn from_writer<W>(w: W) -> result::Result<Data<'static>, WrappedError<W>>
    where W: Write + Send + 'static {
        let cbs = ffi::gpgme_data_cbs {
            read: None,
            write: Some(write_callback::<W>),
            seek: None,
            release: Some(release_callback::<W>),
        };
        unsafe { Data::from_callbacks(cbs, w) }
    }

    pub fn from_seekable_writer<W>(w: W) -> result::Result<Data<'static>, WrappedError<W>>
    where W: Write + Seek + Send + 'static {
        let cbs = ffi::gpgme_data_cbs {
            read: None,
            write: Some(write_callback::<W>),
            seek: Some(seek_callback::<W>),
            release: Some(release_callback::<W>),
        };
        unsafe { Data::from_callbacks(cbs, w) }
    }

    pub fn from_stream<S: Send + 'static>(s: S) -> result::Result<Data<'static>, WrappedError<S>>
    where S: Read + Write {
        let cbs = ffi::gpgme_data_cbs {
            read: Some(read_callback::<S>),
            write: Some(write_callback::<S>),
            seek: None,
            release: Some(release_callback::<S>),
        };
        unsafe { Data::from_callbacks(cbs, s) }
    }

    pub fn from_seekable_stream<S>(s: S) -> result::Result<Data<'static>, WrappedError<S>>
    where S: Read + Write + Seek + Send + 'static {
        let cbs = ffi::gpgme_data_cbs {
            read: Some(read_callback::<S>),
            write: Some(write_callback::<S>),
            seek: Some(seek_callback::<S>),
            release: Some(release_callback::<S>),
        };
        unsafe { Data::from_callbacks(cbs, s) }
    }

    pub fn filename(&self) -> result::Result<&str, Option<Utf8Error>> {
        self.filename_raw().map_or(Err(None), |s| s.to_str().map_err(Some))
    }

    pub fn filename_raw(&self) -> Option<&CStr> {
        unsafe { ffi::gpgme_data_get_file_name(self.raw).as_ref().map(|s| CStr::from_ptr(s)) }
    }

    pub fn clear_filename(&mut self) -> Result<()> {
        unsafe {
            return_err!(ffi::gpgme_data_set_file_name(self.raw, ptr::null()));
        }
        Ok(())
    }

    pub fn set_filename<S: IntoNativeString>(&mut self, name: S) -> Result<()> {
        let name = name.into_native();
        unsafe {
            return_err!(ffi::gpgme_data_set_file_name(self.raw, name.as_ref().as_ptr()));
        }
        Ok(())
    }

    pub fn encoding(&self) -> Encoding {
        unsafe { Encoding::from_raw(ffi::gpgme_data_get_encoding(self.raw)) }
    }

    pub fn set_encoding(&mut self, enc: Encoding) -> Result<()> {
        unsafe { return_err!(ffi::gpgme_data_set_encoding(self.raw, enc.raw())) }
        Ok(())
    }

    pub fn set_flag<S1, S2>(&mut self, name: S1, value: S2) -> Result<()>
    where S1: IntoNativeString, S2: IntoNativeString {
        let name = name.into_native();
        let value = value.into_native();
        unsafe {
            return_err!(ffi::gpgme_data_set_flag(self.raw, name.as_ref().as_ptr(), value.as_ref().as_ptr()));
        }
        Ok(())
    }

    // GPGME_VERSION >= 1.4.3
    pub fn identify(&mut self) -> Type {
        unsafe { Type::from_raw(ffi::gpgme_data_identify(self.raw, 0)) }
    }

    pub fn try_into_bytes(self) -> Option<Vec<u8>> {
        unsafe {
            let mut len = 0;
            let buf = ffi::gpgme_data_release_and_get_mem(self.into_raw(), &mut len);
            if !buf.is_null() {
                Some(slice::from_raw_parts(buf as *const _, len as usize).to_vec())
            } else {
                None
            }
        }
    }
}

impl<'a> Drop for Data<'a> {
    fn drop(&mut self) {
        unsafe {
            ffi::gpgme_data_release(self.raw);
        }
    }
}

impl<'a> Read for Data<'a> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let result = unsafe {
            let (buf, len) = (buf.as_mut_ptr() as *mut _, buf.len());
            ffi::gpgme_data_read(self.raw, buf, len)
        };
        if result >= 0 {
            Ok(result as usize)
        } else {
            Err(Error::last_os_error().into())
        }
    }
}

impl<'a> Write for Data<'a> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let result = unsafe {
            let (buf, len) = (buf.as_ptr() as *const _, buf.len());
            ffi::gpgme_data_write(self.raw, buf, len)
        };
        if result >= 0 {
            Ok(result as usize)
        } else {
            Err(Error::last_os_error().into())
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> Seek for Data<'a> {
    fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
        let (off, whence) = match pos {
            io::SeekFrom::Start(off) => (off as libc::off_t, libc::SEEK_SET),
            io::SeekFrom::End(off) => (off as libc::off_t, libc::SEEK_END),
            io::SeekFrom::Current(off) => (off as libc::off_t, libc::SEEK_CUR),
        };
        let result = unsafe { ffi::gpgme_data_seek(self.raw, off, whence) };
        if result >= 0 {
            Ok(result as u64)
        } else {
            Err(Error::last_os_error().into())
        }
    }
}

extern "C" fn read_callback<S: Read>(handle: *mut libc::c_void, buffer: *mut libc::c_void,
    size: libc::size_t)
    -> libc::ssize_t {
    let handle = handle as *mut CallbackWrapper<S>;
    unsafe {
        let slice = slice::from_raw_parts_mut(buffer as *mut u8, size as usize);
        (*handle).inner.read(slice).map(|n| n as libc::ssize_t).unwrap_or_else(|err| {
            ffi::gpgme_err_set_errno(Error::from(err).to_errno());
            -1
        })
    }
}

extern "C" fn write_callback<S: Write>(handle: *mut libc::c_void, buffer: *const libc::c_void,
    size: libc::size_t)
    -> libc::ssize_t {
    let handle = handle as *mut CallbackWrapper<S>;
    unsafe {
        let slice = slice::from_raw_parts(buffer as *const u8, size as usize);
        (*handle).inner.write(slice).map(|n| n as libc::ssize_t).unwrap_or_else(|err| {
            ffi::gpgme_err_set_errno(Error::from(err).to_errno());
            -1
        })
    }
}

extern "C" fn seek_callback<S: Seek>(handle: *mut libc::c_void, offset: libc::off_t,
    whence: libc::c_int)
    -> libc::off_t {
    let handle = handle as *mut CallbackWrapper<S>;
    let pos = match whence {
        libc::SEEK_SET => io::SeekFrom::Start(offset as u64),
        libc::SEEK_END => io::SeekFrom::End(offset as i64),
        libc::SEEK_CUR => io::SeekFrom::Current(offset as i64),
        _ => unsafe {
            ffi::gpgme_err_set_errno(ffi::gpgme_err_code_to_errno(error::GPG_ERR_EINVAL));
            return -1 as libc::off_t;
        },
    };
    unsafe {
        (*handle).inner.seek(pos).map(|n| n as libc::off_t).unwrap_or_else(|err| {
            ffi::gpgme_err_set_errno(Error::from(err).to_errno());
            -1
        })
    }
}

extern "C" fn release_callback<S>(handle: *mut libc::c_void) {
    unsafe {
        drop(Box::from_raw(handle as *mut CallbackWrapper<S>));
    }
}
