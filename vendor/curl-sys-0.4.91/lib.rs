#![allow(bad_style)]
#![doc(html_root_url = "https://docs.rs/curl-sys/0.4")]

// These `extern crate` are required for conditional linkages of curl.
#[cfg(link_brotli)]
extern crate brotlic_sys;
#[cfg(link_libnghttp2)]
extern crate libnghttp2_sys;
#[cfg(link_libz)]
extern crate libz_sys;
#[cfg(link_openssl)]
extern crate openssl_sys;
#[cfg(feature = "rustls")]
extern crate rustls_ffi;

use libc::c_ulong;
use libc::{c_char, c_double, c_int, c_long, c_short, c_uint, c_void, size_t, time_t};

#[cfg(unix)]
pub use libc::fd_set;
#[cfg(windows)]
pub use windows_sys::Win32::Networking::WinSock::FD_SET as fd_set;
#[cfg(windows)]
use windows_sys::Win32::Networking::WinSock::SOCKADDR;

#[cfg(target_env = "msvc")]
#[doc(hidden)]
pub type __enum_ty = libc::c_int;
#[cfg(not(target_env = "msvc"))]
#[doc(hidden)]
pub type __enum_ty = libc::c_uint;

pub type CURLINFO = __enum_ty;
pub type CURLoption = __enum_ty;
pub type CURLcode = __enum_ty;
pub type CURLversion = __enum_ty;
pub type curl_off_t = i64;

pub enum CURL {}

#[cfg(unix)]
pub type curl_socket_t = libc::c_int;
#[cfg(unix)]
pub const CURL_SOCKET_BAD: curl_socket_t = -1;
#[cfg(all(windows, target_pointer_width = "32"))]
pub type curl_socket_t = libc::c_uint;
#[cfg(all(windows, target_pointer_width = "64"))]
pub type curl_socket_t = u64;
#[cfg(windows)]
pub const CURL_SOCKET_BAD: curl_socket_t = !0;

pub enum curl_httppost {
    // Note that this changed in some versions of libcurl, so we currently don't
    // bind the fields as they're apparently not stable.
    // pub next: *mut curl_httppost,
    // pub name: *mut c_char,
    // pub namelength: c_long,
    // pub contents: *mut c_char,
    // pub contentslength: c_long,
    // pub buffer: *mut c_char,
    // pub bufferlength: c_long,
    // pub contenttype: *mut c_char,
    // pub contentheader: *mut curl_slist,
    // pub more: *mut curl_httppost,
    // pub flags: c_long,
    // pub showfilename: *mut c_char,
    // pub userp: *mut c_void,
}

// pub const HTTPPOST_FILENAME: c_long = 1 << 0;
// pub const HTTPPOST_READFILE: c_long = 1 << 1;
// pub const HTTPPOST_PTRNAME: c_long = 1 << 2;
// pub const HTTPPOST_PTRCONTENTS: c_long = 1 << 3;
// pub const HTTPPOST_BUFFER: c_long = 1 << 4;
// pub const HTTPPOST_PTRBUFFER: c_long = 1 << 5;
// pub const HTTPPOST_CALLBACK: c_long = 1 << 6;

pub type curl_progress_callback =
    extern "C" fn(*mut c_void, c_double, c_double, c_double, c_double) -> c_int;
// pub type curl_xferinfo_callback = extern fn(*mut c_void,
//                                             curl_off_t,
//                                             curl_off_t,
//                                             curl_off_t,
//                                             curl_off_t) -> c_int;

pub const CURL_WRITEFUNC_PAUSE: size_t = 0x10000001;

pub type curl_write_callback = extern "C" fn(*mut c_char, size_t, size_t, *mut c_void) -> size_t;

pub type curlfiletype = __enum_ty;
pub const CURLFILETYPE_FILE: curlfiletype = 0;
pub const CURLFILETYPE_DIRECTORY: curlfiletype = 1;
pub const CURLFILETYPE_SYMLINK: curlfiletype = 2;
pub const CURLFILETYPE_DEVICE_BLOCK: curlfiletype = 3;
pub const CURLFILETYPE_DEVICE_CHAR: curlfiletype = 4;
pub const CURLFILETYPE_NAMEDPIPE: curlfiletype = 5;
pub const CURLFILETYPE_SOCKET: curlfiletype = 6;
pub const CURLFILETYPE_DOOR: curlfiletype = 7;
pub const CURLFILETYPE_UNKNOWN: curlfiletype = 8;

pub const CURLFINFOFLAG_KNOWN_FILENAME: c_uint = 1 << 0;
pub const CURLFINFOFLAG_KNOWN_FILETYPE: c_uint = 1 << 1;
pub const CURLFINFOFLAG_KNOWN_TIME: c_uint = 1 << 2;
pub const CURLFINFOFLAG_KNOWN_PERM: c_uint = 1 << 3;
pub const CURLFINFOFLAG_KNOWN_UID: c_uint = 1 << 4;
pub const CURLFINFOFLAG_KNOWN_GID: c_uint = 1 << 5;
pub const CURLFINFOFLAG_KNOWN_SIZE: c_uint = 1 << 6;
pub const CURLFINFOFLAG_KNOWN_HLINKCOUNT: c_uint = 1 << 7;

#[repr(C)]
pub struct curl_fileinfo {
    pub filename: *mut c_char,
    pub filetype: curlfiletype,
    pub time: time_t,
    pub perm: c_uint,
    pub uid: c_int,
    pub gid: c_int,
    pub size: curl_off_t,
    pub hardlinks: c_long,

    pub strings_time: *mut c_char,
    pub strings_perm: *mut c_char,
    pub strings_user: *mut c_char,
    pub strings_group: *mut c_char,
    pub strings_target: *mut c_char,

    pub flags: c_uint,
    pub b_data: *mut c_char,
    pub b_size: size_t,
    pub b_used: size_t,
}

pub const CURL_BLOB_NOCOPY: c_uint = 0;
pub const CURL_BLOB_COPY: c_uint = 1;

#[repr(C)]
pub struct curl_blob {
    pub data: *mut c_void,
    pub len: size_t,
    pub flags: c_uint,
}

/// Private Moli extension; each observation uses the installed monotonic clock.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct curl_moli_connection_timing {
    pub dns_start: curl_off_t,
    pub dns_end: curl_off_t,
    pub has_dns: curl_off_t,
    pub connect_start: curl_off_t,
    pub connect_end: curl_off_t,
    pub secure_start: curl_off_t,
}

pub const CURL_CHUNK_BGN_FUNC_OK: c_long = 0;
pub const CURL_CHUNK_BGN_FUNC_FAIL: c_long = 1;
pub const CURL_CHUNK_BGN_FUNC_SKIP: c_long = 2;
pub type curl_chunk_bgn_callback = extern "C" fn(*const c_void, *mut c_void, c_int) -> c_long;

pub const CURL_CHUNK_END_FUNC_OK: c_long = 0;
pub const CURL_CHUNK_END_FUNC_FAIL: c_long = 1;
pub type curl_chunk_end_callback = extern "C" fn(*mut c_void) -> c_long;

pub const CURL_FNMATCHFUNC_MATCH: c_int = 0;
pub const CURL_FNMATCHFUNC_NOMATCH: c_int = 1;
pub const CURL_FNMATCHFUNC_FAIL: c_int = 2;
pub type curl_fnmatch_callback = extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int;

pub const CURL_SEEKFUNC_OK: c_int = 0;
pub const CURL_SEEKFUNC_FAIL: c_int = 1;
pub const CURL_SEEKFUNC_CANTSEEK: c_int = 2;
pub type curl_seek_callback = extern "C" fn(*mut c_void, curl_off_t, c_int) -> c_int;

pub const CURL_READFUNC_ABORT: size_t = 0x10000000;
pub const CURL_READFUNC_PAUSE: size_t = 0x10000001;
pub type curl_read_callback = extern "C" fn(*mut c_char, size_t, size_t, *mut c_void) -> size_t;

// pub const CURL_SOCKOPT_OK: c_int = 0;
// pub const CURL_SOCKOPT_ERROR: c_int = 1;
// pub const CURL_SOCKOPT_ALREADY_CONNECTED: c_int = 2;
// pub type curl_sockopt_callback = extern fn(*mut c_void,
//                                            curl_socket_t,
//                                            curlsocktype) -> c_int;

pub type curlioerr = __enum_ty;
pub const CURLIOE_OK: curlioerr = 0;
pub const CURLIOE_UNKNOWNCMD: curlioerr = 1;
pub const CURLIOE_FAILRESTART: curlioerr = 2;

pub type curliocmd = __enum_ty;
pub const CURLIOCMD_NOP: curliocmd = 0;
pub const CURLIOCMD_RESTARTREAD: curliocmd = 1;

pub type curl_ioctl_callback = extern "C" fn(*mut CURL, c_int, *mut c_void) -> curlioerr;

pub type curl_malloc_callback = extern "C" fn(size_t) -> *mut c_void;
pub type curl_free_callback = extern "C" fn(*mut c_void);
pub type curl_realloc_callback = extern "C" fn(*mut c_void, size_t) -> *mut c_void;
pub type curl_strdup_callback = extern "C" fn(*const c_char) -> *mut c_char;
pub type curl_calloc_callback = extern "C" fn(size_t, size_t) -> *mut c_void;

pub type curl_infotype = __enum_ty;
pub const CURLINFO_TEXT: curl_infotype = 0;
pub const CURLINFO_HEADER_IN: curl_infotype = 1;
pub const CURLINFO_HEADER_OUT: curl_infotype = 2;
pub const CURLINFO_DATA_IN: curl_infotype = 3;
pub const CURLINFO_DATA_OUT: curl_infotype = 4;
pub const CURLINFO_SSL_DATA_IN: curl_infotype = 5;
pub const CURLINFO_SSL_DATA_OUT: curl_infotype = 6;

pub type curl_debug_callback =
    extern "C" fn(*mut CURL, curl_infotype, *mut c_char, size_t, *mut c_void) -> c_int;

pub const CURL_PREREQFUNC_OK: c_int = 0;
pub const CURL_PREREQFUNC_ABORT: c_int = 1;
pub type curl_prereq_callback =
    extern "C" fn(*mut c_void, *mut c_char, *mut c_char, c_int, c_int) -> c_int;

pub const CURLE_OK: CURLcode = 0;
pub const CURLE_UNSUPPORTED_PROTOCOL: CURLcode = 1;
pub const CURLE_FAILED_INIT: CURLcode = 2;
pub const CURLE_URL_MALFORMAT: CURLcode = 3;
// pub const CURLE_NOT_BUILT_IN: CURLcode = 4;
pub const CURLE_COULDNT_RESOLVE_PROXY: CURLcode = 5;
pub const CURLE_COULDNT_RESOLVE_HOST: CURLcode = 6;
pub const CURLE_COULDNT_CONNECT: CURLcode = 7;
pub const CURLE_FTP_WEIRD_SERVER_REPLY: CURLcode = 8;
pub const CURLE_REMOTE_ACCESS_DENIED: CURLcode = 9;
// pub const CURLE_FTP_ACCEPT_FAILED: CURLcode = 10;
pub const CURLE_FTP_WEIRD_PASS_REPLY: CURLcode = 11;
// pub const CURLE_FTP_ACCEPT_TIMEOUT: CURLcode = 12;
pub const CURLE_FTP_WEIRD_PASV_REPLY: CURLcode = 13;
pub const CURLE_FTP_WEIRD_227_FORMAT: CURLcode = 14;
pub const CURLE_FTP_CANT_GET_HOST: CURLcode = 15;
pub const CURLE_HTTP2: CURLcode = 16;
pub const CURLE_FTP_COULDNT_SET_TYPE: CURLcode = 17;
pub const CURLE_PARTIAL_FILE: CURLcode = 18;
pub const CURLE_FTP_COULDNT_RETR_FILE: CURLcode = 19;
pub const CURLE_OBSOLETE20: CURLcode = 20;
pub const CURLE_QUOTE_ERROR: CURLcode = 21;
pub const CURLE_HTTP_RETURNED_ERROR: CURLcode = 22;
pub const CURLE_WRITE_ERROR: CURLcode = 23;
pub const CURLE_OBSOLETE24: CURLcode = 24;
pub const CURLE_UPLOAD_FAILED: CURLcode = 25;
pub const CURLE_READ_ERROR: CURLcode = 26;
pub const CURLE_OUT_OF_MEMORY: CURLcode = 27;
pub const CURLE_OPERATION_TIMEDOUT: CURLcode = 28;
pub const CURLE_OBSOLETE29: CURLcode = 29;
pub const CURLE_FTP_PORT_FAILED: CURLcode = 30;
pub const CURLE_FTP_COULDNT_USE_REST: CURLcode = 31;
pub const CURLE_OBSOLETE32: CURLcode = 32;
pub const CURLE_RANGE_ERROR: CURLcode = 33;
pub const CURLE_HTTP_POST_ERROR: CURLcode = 34;
pub const CURLE_SSL_CONNECT_ERROR: CURLcode = 35;
pub const CURLE_BAD_DOWNLOAD_RESUME: CURLcode = 36;
pub const CURLE_FILE_COULDNT_READ_FILE: CURLcode = 37;
pub const CURLE_LDAP_CANNOT_BIND: CURLcode = 38;
pub const CURLE_LDAP_SEARCH_FAILED: CURLcode = 39;
pub const CURLE_OBSOLETE40: CURLcode = 40;
pub const CURLE_FUNCTION_NOT_FOUND: CURLcode = 41;
pub const CURLE_ABORTED_BY_CALLBACK: CURLcode = 42;
pub const CURLE_BAD_FUNCTION_ARGUMENT: CURLcode = 43;
pub const CURLE_OBSOLETE44: CURLcode = 44;
pub const CURLE_INTERFACE_FAILED: CURLcode = 45;
pub const CURLE_OBSOLETE46: CURLcode = 46;
pub const CURLE_TOO_MANY_REDIRECTS: CURLcode = 47;
pub const CURLE_UNKNOWN_OPTION: CURLcode = 48;
pub const CURLE_TELNET_OPTION_SYNTAX: CURLcode = 49;
pub const CURLE_OBSOLETE50: CURLcode = 50;
pub const CURLE_PEER_FAILED_VERIFICATION: CURLcode = 60;
pub const CURLE_GOT_NOTHING: CURLcode = 52;
pub const CURLE_SSL_ENGINE_NOTFOUND: CURLcode = 53;
pub const CURLE_SSL_ENGINE_SETFAILED: CURLcode = 54;
pub const CURLE_SEND_ERROR: CURLcode = 55;
pub const CURLE_RECV_ERROR: CURLcode = 56;
pub const CURLE_OBSOLETE57: CURLcode = 57;
pub const CURLE_SSL_CERTPROBLEM: CURLcode = 58;
pub const CURLE_SSL_CIPHER: CURLcode = 59;
pub const CURLE_SSL_CACERT: CURLcode = 60;
pub const CURLE_BAD_CONTENT_ENCODING: CURLcode = 61;
pub const CURLE_LDAP_INVALID_URL: CURLcode = 62;
pub const CURLE_FILESIZE_EXCEEDED: CURLcode = 63;
pub const CURLE_USE_SSL_FAILED: CURLcode = 64;
pub const CURLE_SEND_FAIL_REWIND: CURLcode = 65;
pub const CURLE_SSL_ENGINE_INITFAILED: CURLcode = 66;
pub const CURLE_LOGIN_DENIED: CURLcode = 67;
pub const CURLE_TFTP_NOTFOUND: CURLcode = 68;
pub const CURLE_TFTP_PERM: CURLcode = 69;
pub const CURLE_REMOTE_DISK_FULL: CURLcode = 70;
pub const CURLE_TFTP_ILLEGAL: CURLcode = 71;
pub const CURLE_TFTP_UNKNOWNID: CURLcode = 72;
pub const CURLE_REMOTE_FILE_EXISTS: CURLcode = 73;
pub const CURLE_TFTP_NOSUCHUSER: CURLcode = 74;
pub const CURLE_CONV_FAILED: CURLcode = 75;
pub const CURLE_CONV_REQD: CURLcode = 76;
pub const CURLE_SSL_CACERT_BADFILE: CURLcode = 77;
pub const CURLE_REMOTE_FILE_NOT_FOUND: CURLcode = 78;
pub const CURLE_SSH: CURLcode = 79;
pub const CURLE_SSL_SHUTDOWN_FAILED: CURLcode = 80;
pub const CURLE_AGAIN: CURLcode = 81;
pub const CURLE_SSL_CRL_BADFILE: CURLcode = 82;
pub const CURLE_SSL_ISSUER_ERROR: CURLcode = 83;
pub const CURLE_FTP_PRET_FAILED: CURLcode = 84;
pub const CURLE_RTSP_CSEQ_ERROR: CURLcode = 85;
pub const CURLE_RTSP_SESSION_ERROR: CURLcode = 86;
pub const CURLE_FTP_BAD_FILE_LIST: CURLcode = 87;
pub const CURLE_CHUNK_FAILED: CURLcode = 88;
pub const CURLE_NO_CONNECTION_AVAILABLE: CURLcode = 89;
pub const CURLE_SSL_PINNEDPUBKEYNOTMATCH: CURLcode = 90;
pub const CURLE_SSL_INVALIDCERTSTATUS: CURLcode = 91;
pub const CURLE_HTTP2_STREAM: CURLcode = 92;
pub const CURLE_RECURSIVE_API_CALL: CURLcode = 93;

pub type curl_conv_callback = extern "C" fn(*mut c_char, size_t) -> CURLcode;
pub type curl_ssl_ctx_callback = extern "C" fn(*mut CURL, *mut c_void, *mut c_void) -> CURLcode;

pub type curl_proxytype = __enum_ty;
pub const CURLPROXY_HTTP: curl_proxytype = 0;
pub const CURLPROXY_HTTP_1_0: curl_proxytype = 1;
pub const CURLPROXY_SOCKS4: curl_proxytype = 4;
pub const CURLPROXY_SOCKS5: curl_proxytype = 5;
pub const CURLPROXY_SOCKS4A: curl_proxytype = 6;
pub const CURLPROXY_SOCKS5_HOSTNAME: curl_proxytype = 7;

pub const CURLAUTH_NONE: c_ulong = 0;
pub const CURLAUTH_BASIC: c_ulong = 1 << 0;
pub const CURLAUTH_DIGEST: c_ulong = 1 << 1;
pub const CURLAUTH_GSSNEGOTIATE: c_ulong = 1 << 2;
pub const CURLAUTH_NTLM: c_ulong = 1 << 3;
pub const CURLAUTH_DIGEST_IE: c_ulong = 1 << 4;
pub const CURLAUTH_NTLM_WB: c_ulong = 1 << 5;
pub const CURLAUTH_AWS_SIGV4: c_ulong = 1 << 7;
pub const CURLAUTH_HTTPSIG: c_ulong = 1 << 8;
// pub const CURLAUTH_ONLY: c_ulong = 1 << 31;
pub const CURLAUTH_ANY: c_ulong = (!(CURLAUTH_DIGEST_IE | CURLAUTH_HTTPSIG)) & 0xffffffff;
pub const CURLAUTH_ANYSAFE: c_ulong =
    (!(CURLAUTH_BASIC | CURLAUTH_DIGEST_IE | CURLAUTH_HTTPSIG)) & 0xffffffff;

pub const CURLHTTPSIG_NONE: c_long = 0;
pub const CURLHTTPSIG_ED25519: c_long = 1;
pub const CURLHTTPSIG_HMAC_SHA256: c_long = 2;

// pub const CURLSSH_AUTH_ANY: c_ulong = !0;
// pub const CURLSSH_AUTH_NONE: c_ulong = 0;
// pub const CURLSSH_AUTH_PUBLICKEY: c_ulong = 1 << 0;
// pub const CURLSSH_AUTH_PASSWORD: c_ulong = 1 << 1;
// pub const CURLSSH_AUTH_HOST: c_ulong = 1 << 2;
// pub const CURLSSH_AUTH_KEYBOARD: c_ulong = 1 << 3;
// pub const CURLSSH_AUTH_AGENT: c_ulong = 1 << 4;
// pub const CURLSSH_AUTH_DEFAULT: c_ulong = CURLSSH_AUTH_ANY;

pub const CURLGSSAPI_DELEGATION_NONE: c_ulong = 0;
pub const CURLGSSAPI_DELEGATION_POLICY_FLAG: c_ulong = 1 << 0;
pub const CURLGSSAPI_DELEGATION_FLAG: c_ulong = 1 << 1;

// pub type curl_khtype = __enum_ty;
// pub const CURLKHTYPE_UNKNOWN: curl_khtype = 0;
// pub const CURLKHTYPE_RSA1: curl_khtype = 1;
// pub const CURLKHTYPE_RSA: curl_khtype = 2;
// pub const CURLKHTYPE_DSS: curl_khtype = 3;

// #[repr(C)]
// pub struct curl_khkey {
//     pub key: *const c_char,
//     pub len: size_t,
//     pub keytype: curl_khtype,
// }

// pub type curl_khstat = __enum_ty;
// pub const CURLKHSTAT_FINE_ADD_TO_FILE: curl_khstat = 0;
// pub const CURLKHSTAT_FINE: curl_khstat = 1;
// pub const CURLKHSTAT_REJECT: curl_khstat = 2;
// pub const CURLKHSTAT_DEFER: curl_khstat = 3;
//
// pub type curl_khmatch = __enum_ty;
// pub const CURLKHMATCH_OK: curl_khmatch = 0;
// pub const CURLKHMATCH_MISMATCH: curl_khmatch = 1;
// pub const CURLKHMATCH_MISSING: curl_khmatch = 2;

// pub type curl_sshkeycallback = extern fn(*mut CURL,
//                                          *const curl_khkey,
//                                          *const curl_khkey,
//                                          curl_khmatch,
//                                          *mut c_void) -> c_int;

pub const CURL_NETRC_IGNORED: c_ulong = 0;
pub const CURL_NETRC_OPTIONAL: c_ulong = 1;
pub const CURL_NETRC_REQUIRED: c_ulong = 2;

pub type curl_usessl = __enum_ty;
pub const CURLUSESSL_NONE: curl_usessl = 0;
pub const CURLUSESSL_TRY: curl_usessl = 1;
pub const CURLUSESSL_CONTROL: curl_usessl = 2;
pub const CURLUSESSL_ALL: curl_usessl = 3;

pub const CURLPROTO_HTTP: c_int = 1 << 0;
pub const CURLPROTO_HTTPS: c_int = 1 << 1;
pub const CURLPROTO_FILE: c_int = 1 << 10;

pub const CURLOPTTYPE_LONG: CURLoption = 0;
pub const CURLOPTTYPE_OBJECTPOINT: CURLoption = 10_000;
pub const CURLOPTTYPE_FUNCTIONPOINT: CURLoption = 20_000;
pub const CURLOPTTYPE_OFF_T: CURLoption = 30_000;
pub const CURLOPTTYPE_BLOB: CURLoption = 40_000;
const CURLOPTTYPE_VALUES: CURLoption = CURLOPTTYPE_LONG;

pub const CURLOPT_FILE: CURLoption = CURLOPTTYPE_OBJECTPOINT + 1;
pub const CURLOPT_URL: CURLoption = CURLOPTTYPE_OBJECTPOINT + 2;
pub const CURLOPT_PORT: CURLoption = CURLOPTTYPE_LONG + 3;
pub const CURLOPT_PROXY: CURLoption = CURLOPTTYPE_OBJECTPOINT + 4;
pub const CURLOPT_USERPWD: CURLoption = CURLOPTTYPE_OBJECTPOINT + 5;
pub const CURLOPT_PROXYUSERPWD: CURLoption = CURLOPTTYPE_OBJECTPOINT + 6;
pub const CURLOPT_RANGE: CURLoption = CURLOPTTYPE_OBJECTPOINT + 7;
pub const CURLOPT_INFILE: CURLoption = CURLOPTTYPE_OBJECTPOINT + 9;
pub const CURLOPT_ERRORBUFFER: CURLoption = CURLOPTTYPE_OBJECTPOINT + 10;
pub const CURLOPT_WRITEFUNCTION: CURLoption = CURLOPTTYPE_FUNCTIONPOINT + 11;
pub const CURLOPT_READFUNCTION: CURLoption = CURLOPTTYPE_FUNCTIONPOINT + 12;
pub const CURLOPT_TIMEOUT: CURLoption = CURLOPTTYPE_LONG + 13;
pub const CURLOPT_INFILESIZE: CURLoption = CURLOPTTYPE_LONG + 14;
pub const CURLOPT_POSTFIELDS: CURLoption = CURLOPTTYPE_OBJECTPOINT + 15;
pub const CURLOPT_REFERER: CURLoption = CURLOPTTYPE_OBJECTPOINT + 16;
pub const CURLOPT_FTPPORT: CURLoption = CURLOPTTYPE_OBJECTPOINT + 17;
pub const CURLOPT_USERAGENT: CURLoption = CURLOPTTYPE_OBJECTPOINT + 18;
pub const CURLOPT_LOW_SPEED_LIMIT: CURLoption = CURLOPTTYPE_LONG + 19;
pub const CURLOPT_LOW_SPEED_TIME: CURLoption = CURLOPTTYPE_LONG + 20;
pub const CURLOPT_RESUME_FROM: CURLoption = CURLOPTTYPE_LONG + 21;
pub const CURLOPT_COOKIE: CURLoption = CURLOPTTYPE_OBJECTPOINT + 22;
pub const CURLOPT_HTTPHEADER: CURLoption = CURLOPTTYPE_OBJECTPOINT + 23;
pub const CURLOPT_HTTPPOST: CURLoption = CURLOPTTYPE_OBJECTPOINT + 24;
pub const CURLOPT_SSLCERT: CURLoption = CURLOPTTYPE_OBJECTPOINT + 25;
pub const CURLOPT_KEYPASSWD: CURLoption = CURLOPTTYPE_OBJECTPOINT + 26;
pub const CURLOPT_CRLF: CURLoption = CURLOPTTYPE_LONG + 27;
pub const CURLOPT_QUOTE: CURLoption = CURLOPTTYPE_OBJECTPOINT + 28;
pub const CURLOPT_WRITEHEADER: CURLoption = CURLOPTTYPE_OBJECTPOINT + 29;
pub const CURLOPT_COOKIEFILE: CURLoption = CURLOPTTYPE_OBJECTPOINT + 31;
pub const CURLOPT_SSLVERSION: CURLoption = CURLOPTTYPE_LONG + 32;
pub const CURLOPT_TIMECONDITION: CURLoption = CURLOPTTYPE_LONG + 33;
pub const CURLOPT_TIMEVALUE: CURLoption = CURLOPTTYPE_LONG + 34;
pub const CURLOPT_CUSTOMREQUEST: CURLoption = CURLOPTTYPE_OBJECTPOINT + 36;
pub const CURLOPT_STDERR: CURLoption = CURLOPTTYPE_OBJECTPOINT + 37;
pub const CURLOPT_POSTQUOTE: CURLoption = CURLOPTTYPE_OBJECTPOINT + 39;
#[deprecated = "has no effect"]
pub const CURLOPT_WRITEINFO: CURLoption = 9999;
pub const CURLOPT_VERBOSE: CURLoption = CURLOPTTYPE_LONG + 41;
pub const CURLOPT_HEADER: CURLoption = CURLOPTTYPE_LONG + 42;
pub const CURLOPT_NOPROGRESS: CURLoption = CURLOPTTYPE_LONG + 43;
pub const CURLOPT_NOBODY: CURLoption = CURLOPTTYPE_LONG + 44;
pub const CURLOPT_FAILONERROR: CURLoption = CURLOPTTYPE_LONG + 45;
pub const CURLOPT_UPLOAD: CURLoption = CURLOPTTYPE_LONG + 46;
pub const CURLOPT_POST: CURLoption = CURLOPTTYPE_LONG + 47;
pub const CURLOPT_DIRLISTONLY: CURLoption = CURLOPTTYPE_LONG + 48;
pub const CURLOPT_APPEND: CURLoption = CURLOPTTYPE_LONG + 50;
pub const CURLOPT_NETRC: CURLoption = CURLOPTTYPE_LONG + 51;
pub const CURLOPT_FOLLOWLOCATION: CURLoption = CURLOPTTYPE_LONG + 52;
pub const CURLOPT_TRANSFERTEXT: CURLoption = CURLOPTTYPE_LONG + 53;
pub const CURLOPT_PUT: CURLoption = CURLOPTTYPE_LONG + 54;
pub const CURLOPT_PROGRESSFUNCTION: CURLoption = CURLOPTTYPE_FUNCTIONPOINT + 56;
pub const CURLOPT_PROGRESSDATA: CURLoption = CURLOPTTYPE_OBJECTPOINT + 57;
pub const CURLOPT_AUTOREFERER: CURLoption = CURLOPTTYPE_LONG + 58;
pub const CURLOPT_PROXYPORT: CURLoption = CURLOPTTYPE_LONG + 59;
pub const CURLOPT_POSTFIELDSIZE: CURLoption = CURLOPTTYPE_LONG + 60;
pub const CURLOPT_HTTPPROXYTUNNEL: CURLoption = CURLOPTTYPE_LONG + 61;
pub const CURLOPT_INTERFACE: CURLoption = CURLOPTTYPE_OBJECTPOINT + 62;
pub const CURLOPT_KRBLEVEL: CURLoption = CURLOPTTYPE_OBJECTPOINT + 63;
pub const CURLOPT_SSL_VERIFYPEER: CURLoption = CURLOPTTYPE_LONG + 64;
pub const CURLOPT_CAINFO: CURLoption = CURLOPTTYPE_OBJECTPOINT + 65;
pub const CURLOPT_MAXREDIRS: CURLoption = CURLOPTTYPE_LONG + 68;
pub const CURLOPT_FILETIME: CURLoption = CURLOPTTYPE_LONG + 69;
pub const CURLOPT_TELNETOPTIONS: CURLoption = CURLOPTTYPE_OBJECTPOINT + 70;
pub const CURLOPT_MAXCONNECTS: CURLoption = CURLOPTTYPE_LONG + 71;
#[deprecated = "has no effect"]
pub const CURLOPT_CLOSEPOLICY: CURLoption = 9999;
pub const CURLOPT_FRESH_CONNECT: CURLoption = CURLOPTTYPE_LONG + 74;
pub const CURLOPT_FORBID_REUSE: CURLoption = CURLOPTTYPE_LONG + 75;
pub const CURLOPT_RANDOM_FILE: CURLoption = CURLOPTTYPE_OBJECTPOINT + 76;
pub const CURLOPT_EGDSOCKET: CURLoption = CURLOPTTYPE_OBJECTPOINT + 77;
pub const CURLOPT_CONNECTTIMEOUT: CURLoption = CURLOPTTYPE_LONG + 78;
pub const CURLOPT_HEADERFUNCTION: CURLoption = CURLOPTTYPE_FUNCTIONPOINT + 79;
pub const CURLOPT_HTTPGET: CURLoption = CURLOPTTYPE_LONG + 80;
pub const CURLOPT_SSL_VERIFYHOST: CURLoption = CURLOPTTYPE_LONG + 81;
pub const CURLOPT_COOKIEJAR: CURLoption = CURLOPTTYPE_OBJECTPOINT + 82;
pub const CURLOPT_SSL_CIPHER_LIST: CURLoption = CURLOPTTYPE_OBJECTPOINT + 83;
pub const CURLOPT_HTTP_VERSION: CURLoption = CURLOPTTYPE_LONG + 84;
pub const CURLOPT_FTP_USE_EPSV: CURLoption = CURLOPTTYPE_LONG + 85;
pub const CURLOPT_SSLCERTTYPE: CURLoption = CURLOPTTYPE_OBJECTPOINT + 86;
pub const CURLOPT_SSLKEY: CURLoption = CURLOPTTYPE_OBJECTPOINT + 87;
pub const CURLOPT_SSLKEYTYPE: CURLoption = CURLOPTTYPE_OBJECTPOINT + 88;
pub const CURLOPT_SSLENGINE: CURLoption = CURLOPTTYPE_OBJECTPOINT + 89;
pub const CURLOPT_SSLENGINE_DEFAULT: CURLoption = CURLOPTTYPE_LONG + 90;
pub const CURLOPT_DNS_USE_GLOBAL_CACHE: CURLoption = CURLOPTTYPE_LONG + 91;
pub const CURLOPT_DNS_CACHE_TIMEOUT: CURLoption = CURLOPTTYPE_LONG + 92;
pub const CURLOPT_PREQUOTE: CURLoption = CURLOPTTYPE_OBJECTPOINT + 93;
pub const CURLOPT_DEBUGFUNCTION: CURLoption = CURLOPTTYPE_FUNCTIONPOINT + 94;
pub const CURLOPT_DEBUGDATA: CURLoption = CURLOPTTYPE_OBJECTPOINT + 95;
pub const CURLOPT_COOKIESESSION: CURLoption = CURLOPTTYPE_LONG + 96;
pub const CURLOPT_CAPATH: CURLoption = CURLOPTTYPE_OBJECTPOINT + 97;
pub const CURLOPT_BUFFERSIZE: CURLoption = CURLOPTTYPE_LONG + 98;
pub const CURLOPT_NOSIGNAL: CURLoption = CURLOPTTYPE_LONG + 99;
pub const CURLOPT_SHARE: CURLoption = CURLOPTTYPE_OBJECTPOINT + 100;
pub const CURLOPT_PROXYTYPE: CURLoption = CURLOPTTYPE_LONG + 101;
pub const CURLOPT_ACCEPT_ENCODING: CURLoption = CURLOPTTYPE_OBJECTPOINT + 102;
pub const CURLOPT_PRIVATE: CURLoption = CURLOPTTYPE_OBJECTPOINT + 103;
pub const CURLOPT_HTTP200ALIASES: CURLoption = CURLOPTTYPE_OBJECTPOINT + 104;
pub const CURLOPT_UNRESTRICTED_AUTH: CURLoption = CURLOPTTYPE_LONG + 105;
pub const CURLOPT_FTP_USE_EPRT: CURLoption = CURLOPTTYPE_LONG + 106;
pub const CURLOPT_HTTPAUTH: CURLoption = CURLOPTTYPE_LONG + 107;
pub const CURLOPT_SSL_CTX_FUNCTION: CURLoption = CURLOPTTYPE_FUNCTIONPOINT + 108;
pub const CURLOPT_SSL_CTX_DATA: CURLoption = CURLOPTTYPE_OBJECTPOINT + 109;
pub const CURLOPT_FTP_CREATE_MISSING_DIRS: CURLoption = CURLOPTTYPE_LONG + 110;
pub const CURLOPT_PROXYAUTH: CURLoption = CURLOPTTYPE_LONG + 111;
pub const CURLOPT_FTP_RESPONSE_TIMEOUT: CURLoption = CURLOPTTYPE_LONG + 112;
pub const CURLOPT_IPRESOLVE: CURLoption = CURLOPTTYPE_LONG + 113;
pub const CURLOPT_MAXFILESIZE: CURLoption = CURLOPTTYPE_LONG + 114;
pub const CURLOPT_INFILESIZE_LARGE: CURLoption = CURLOPTTYPE_OFF_T + 115;
pub const CURLOPT_RESUME_FROM_LARGE: CURLoption = CURLOPTTYPE_OFF_T + 116;
pub const CURLOPT_MAXFILESIZE_LARGE: CURLoption = CURLOPTTYPE_OFF_T + 117;
pub const CURLOPT_NETRC_FILE: CURLoption = CURLOPTTYPE_OBJECTPOINT + 118;
pub const CURLOPT_USE_SSL: CURLoption = CURLOPTTYPE_LONG + 119;
pub const CURLOPT_POSTFIELDSIZE_LARGE: CURLoption = CURLOPTTYPE_OFF_T + 120;
pub const CURLOPT_TCP_NODELAY: CURLoption = CURLOPTTYPE_LONG + 121;
pub const CURLOPT_FTPSSLAUTH: CURLoption = CURLOPTTYPE_LONG + 129;
pub const CURLOPT_IOCTLFUNCTION: CURLoption = CURLOPTTYPE_FUNCTIONPOINT + 130;
pub const CURLOPT_IOCTLDATA: CURLoption = CURLOPTTYPE_OBJECTPOINT + 131;
pub const CURLOPT_FTP_ACCOUNT: CURLoption = CURLOPTTYPE_OBJECTPOINT + 134;
pub const CURLOPT_COOKIELIST: CURLoption = CURLOPTTYPE_OBJECTPOINT + 135;
pub const CURLOPT_IGNORE_CONTENT_LENGTH: CURLoption = CURLOPTTYPE_LONG + 136;
pub const CURLOPT_FTP_SKIP_PASV_IP: CURLoption = CURLOPTTYPE_LONG + 137;
pub const CURLOPT_FTP_FILEMETHOD: CURLoption = CURLOPTTYPE_LONG + 138;
pub const CURLOPT_LOCALPORT: CURLoption = CURLOPTTYPE_LONG + 139;
pub const CURLOPT_LOCALPORTRANGE: CURLoption = CURLOPTTYPE_LONG + 140;
pub const CURLOPT_CONNECT_ONLY: CURLoption = CURLOPTTYPE_LONG + 141;
pub const CURLOPT_CONV_FROM_NETWORK_FUNCTION: CURLoption = CURLOPTTYPE_FUNCTIONPOINT + 142;
pub const CURLOPT_CONV_TO_NETWORK_FUNCTION: CURLoption = CURLOPTTYPE_FUNCTIONPOINT + 143;
pub const CURLOPT_CONV_FROM_UTF8_FUNCTION: CURLoption = CURLOPTTYPE_FUNCTIONPOINT + 144;
pub const CURLOPT_MAX_SEND_SPEED_LARGE: CURLoption = CURLOPTTYPE_OFF_T + 145;
pub const CURLOPT_MAX_RECV_SPEED_LARGE: CURLoption = CURLOPTTYPE_OFF_T + 146;
pub const CURLOPT_FTP_ALTERNATIVE_TO_USER: CURLoption = CURLOPTTYPE_OBJECTPOINT + 147;
pub const CURLOPT_SOCKOPTFUNCTION: CURLoption = CURLOPTTYPE_FUNCTIONPOINT + 148;
pub const CURLOPT_SOCKOPTDATA: CURLoption = CURLOPTTYPE_OBJECTPOINT + 149;
pub const CURLOPT_SSL_SESSIONID_CACHE: CURLoption = CURLOPTTYPE_LONG + 150;
pub const CURLOPT_SSH_AUTH_TYPES: CURLoption = CURLOPTTYPE_LONG + 151;
pub const CURLOPT_SSH_PUBLIC_KEYFILE: CURLoption = CURLOPTTYPE_OBJECTPOINT + 152;
pub const CURLOPT_SSH_PRIVATE_KEYFILE: CURLoption = CURLOPTTYPE_OBJECTPOINT + 153;
pub const CURLOPT_FTP_SSL_CCC: CURLoption = CURLOPTTYPE_LONG + 154;
pub const CURLOPT_TIMEOUT_MS: CURLoption = CURLOPTTYPE_LONG + 155;
pub const CURLOPT_CONNECTTIMEOUT_MS: CURLoption = CURLOPTTYPE_LONG + 156;
pub const CURLOPT_HTTP_TRANSFER_DECODING: CURLoption = CURLOPTTYPE_LONG + 157;
pub const CURLOPT_HTTP_CONTENT_DECODING: CURLoption = CURLOPTTYPE_LONG + 158;
pub const CURLOPT_NEW_FILE_PERMS: CURLoption = CURLOPTTYPE_LONG + 159;
pub const CURLOPT_NEW_DIRECTORY_PERMS: CURLoption = CURLOPTTYPE_LONG + 160;
pub const CURLOPT_POSTREDIR: CURLoption = CURLOPTTYPE_LONG + 161;
pub const CURLOPT_SSH_HOST_PUBLIC_KEY_MD5: CURLoption = CURLOPTTYPE_OBJECTPOINT + 162;
pub const CURLOPT_OPENSOCKETFUNCTION: CURLoption = CURLOPTTYPE_FUNCTIONPOINT + 163;
pub const CURLOPT_OPENSOCKETDATA: CURLoption = CURLOPTTYPE_OBJECTPOINT + 164;
pub const CURLOPT_COPYPOSTFIELDS: CURLoption = CURLOPTTYPE_OBJECTPOINT + 165;
pub const CURLOPT_PROXY_TRANSFER_MODE: CURLoption = CURLOPTTYPE_LONG + 166;
pub const CURLOPT_SEEKFUNCTION: CURLoption = CURLOPTTYPE_FUNCTIONPOINT + 167;
pub const CURLOPT_SEEKDATA: CURLoption = CURLOPTTYPE_OBJECTPOINT + 168;
pub const CURLOPT_CRLFILE: CURLoption = CURLOPTTYPE_OBJECTPOINT + 169;
pub const CURLOPT_ISSUERCERT: CURLoption = CURLOPTTYPE_OBJECTPOINT + 170;
pub const CURLOPT_ADDRESS_SCOPE: CURLoption = CURLOPTTYPE_LONG + 171;
pub const CURLOPT_CERTINFO: CURLoption = CURLOPTTYPE_LONG + 172;
pub const CURLOPT_USERNAME: CURLoption = CURLOPTTYPE_OBJECTPOINT + 173;
pub const CURLOPT_PASSWORD: CURLoption = CURLOPTTYPE_OBJECTPOINT + 174;
pub const CURLOPT_PROXYUSERNAME: CURLoption = CURLOPTTYPE_OBJECTPOINT + 175;
pub const CURLOPT_PROXYPASSWORD: CURLoption = CURLOPTTYPE_OBJECTPOINT + 176;
pub const CURLOPT_NOPROXY: CURLoption = CURLOPTTYPE_OBJECTPOINT + 177;
pub const CURLOPT_TFTP_BLKSIZE: CURLoption = CURLOPTTYPE_LONG + 178;
pub const CURLOPT_SOCKS5_GSSAPI_SERVICE: CURLoption = CURLOPTTYPE_OBJECTPOINT + 179;
pub const CURLOPT_SOCKS5_GSSAPI_NEC: CURLoption = CURLOPTTYPE_LONG + 180;
pub const CURLOPT_PROTOCOLS: CURLoption = CURLOPTTYPE_LONG + 181;
pub const CURLOPT_REDIR_PROTOCOLS: CURLoption = CURLOPTTYPE_LONG + 182;
pub const CURLOPT_SSH_KNOWNHOSTS: CURLoption = CURLOPTTYPE_OBJECTPOINT + 183;
pub const CURLOPT_SSH_KEYFUNCTION: CURLoption = CURLOPTTYPE_FUNCTIONPOINT + 184;
pub const CURLOPT_SSH_KEYDATA: CURLoption = CURLOPTTYPE_OBJECTPOINT + 185;
pub const CURLOPT_MAIL_FROM: CURLoption = CURLOPTTYPE_OBJECTPOINT + 186;
pub const CURLOPT_MAIL_RCPT: CURLoption = CURLOPTTYPE_OBJECTPOINT + 187;
pub const CURLOPT_FTP_USE_PRET: CURLoption = CURLOPTTYPE_LONG + 188;
pub const CURLOPT_RTSP_REQUEST: CURLoption = CURLOPTTYPE_LONG + 189;
pub const CURLOPT_RTSP_SESSION_ID: CURLoption = CURLOPTTYPE_OBJECTPOINT + 190;
pub const CURLOPT_RTSP_STREAM_URI: CURLoption = CURLOPTTYPE_OBJECTPOINT + 191;
pub const CURLOPT_RTSP_TRANSPORT: CURLoption = CURLOPTTYPE_OBJECTPOINT + 192;
pub const CURLOPT_RTSP_CLIENT_CSEQ: CURLoption = CURLOPTTYPE_LONG + 193;
pub const CURLOPT_RTSP_SERVER_CSEQ: CURLoption = CURLOPTTYPE_LONG + 194;
pub const CURLOPT_INTERLEAVEDATA: CURLoption = CURLOPTTYPE_OBJECTPOINT + 195;
pub const CURLOPT_INTERLEAVEFUNCTION: CURLoption = CURLOPTTYPE_FUNCTIONPOINT + 196;
pub const CURLOPT_WILDCARDMATCH: CURLoption = CURLOPTTYPE_LONG + 197;
pub const CURLOPT_CHUNK_BGN_FUNCTION: CURLoption = CURLOPTTYPE_FUNCTIONPOINT + 198;
pub const CURLOPT_CHUNK_END_FUNCTION: CURLoption = CURLOPTTYPE_FUNCTIONPOINT + 199;
pub const CURLOPT_FNMATCH_FUNCTION: CURLoption = CURLOPTTYPE_FUNCTIONPOINT + 200;
pub const CURLOPT_CHUNK_DATA: CURLoption = CURLOPTTYPE_OBJECTPOINT + 201;
pub const CURLOPT_FNMATCH_DATA: CURLoption = CURLOPTTYPE_OBJECTPOINT + 202;
pub const CURLOPT_RESOLVE: CURLoption = CURLOPTTYPE_OBJECTPOINT + 203;
pub const CURLOPT_TLSAUTH_USERNAME: CURLoption = CURLOPTTYPE_OBJECTPOINT + 204;
pub const CURLOPT_TLSAUTH_PASSWORD: CURLoption = CURLOPTTYPE_OBJECTPOINT + 205;
pub const CURLOPT_TLSAUTH_TYPE: CURLoption = CURLOPTTYPE_OBJECTPOINT + 206;
pub const CURLOPT_TRANSFER_ENCODING: CURLoption = CURLOPTTYPE_LONG + 207;
pub const CURLOPT_CLOSESOCKETFUNCTION: CURLoption = CURLOPTTYPE_FUNCTIONPOINT + 208;
pub const CURLOPT_CLOSESOCKETDATA: CURLoption = CURLOPTTYPE_OBJECTPOINT + 209;
pub const CURLOPT_GSSAPI_DELEGATION: CURLoption = CURLOPTTYPE_LONG + 210;
pub const CURLOPT_DNS_SERVERS: CURLoption = CURLOPTTYPE_OBJECTPOINT + 211;
// pub const CURLOPT_ACCEPTTIMEOUT_MS: CURLoption = CURLOPTTYPE_LONG + 212;
pub const CURLOPT_TCP_KEEPALIVE: CURLoption = CURLOPTTYPE_LONG + 213;
pub const CURLOPT_TCP_KEEPIDLE: CURLoption = CURLOPTTYPE_LONG + 214;
pub const CURLOPT_TCP_KEEPINTVL: CURLoption = CURLOPTTYPE_LONG + 215;
pub const CURLOPT_SSL_OPTIONS: CURLoption = CURLOPTTYPE_LONG + 216;
// pub const CURLOPT_MAIL_AUTH: CURLoption = CURLOPTTYPE_OBJECTPOINT + 217;
// pub const CURLOPT_SASL_IR: CURLoption = CURLOPTTYPE_LONG + 218;
// pub const CURLOPT_XFERINFOFUNCTION: CURLoption = CURLOPTTYPE_FUNCTIONPOINT + 219;
// pub const CURLOPT_XOAUTH2_BEARER: CURLoption = CURLOPTTYPE_OBJECTPOINT + 220;
// pub const CURLOPT_DNS_INTERFACE: CURLoption = CURLOPTTYPE_OBJECTPOINT + 221;
// pub const CURLOPT_DNS_LOCAL_IP4: CURLoption = CURLOPTTYPE_OBJECTPOINT + 222;
// pub const CURLOPT_DNS_LOCAL_IP6: CURLoption = CURLOPTTYPE_OBJECTPOINT + 223;
// pub const CURLOPT_LOGIN_OPTIONS: CURLoption = CURLOPTTYPE_OBJECTPOINT + 224;
pub const CURLOPT_EXPECT_100_TIMEOUT_MS: CURLoption = CURLOPTTYPE_LONG + 227;
pub const CURLOPT_PROXYHEADER: CURLoption = CURLOPTTYPE_OBJECTPOINT + 228;
pub const CURLOPT_HEADEROPT: CURLoption = CURLOPTTYPE_LONG + 229;
pub const CURLOPT_PINNEDPUBLICKEY: CURLoption = CURLOPTTYPE_OBJECTPOINT + 230;
pub const CURLOPT_UNIX_SOCKET_PATH: CURLoption = CURLOPTTYPE_OBJECTPOINT + 231;
pub const CURLOPT_PATH_AS_IS: CURLoption = CURLOPTTYPE_LONG + 234;
pub const CURLOPT_PIPEWAIT: CURLoption = CURLOPTTYPE_LONG + 237;
pub const CURLOPT_CONNECT_TO: CURLoption = CURLOPTTYPE_OBJECTPOINT + 243;
pub const CURLOPT_PROXY_CAINFO: CURLoption = CURLOPTTYPE_OBJECTPOINT + 246;
pub const CURLOPT_PROXY_CAPATH: CURLoption = CURLOPTTYPE_OBJECTPOINT + 247;
pub const CURLOPT_PROXY_SSL_VERIFYPEER: CURLoption = CURLOPTTYPE_LONG + 248;
pub const CURLOPT_PROXY_SSL_VERIFYHOST: CURLoption = CURLOPTTYPE_LONG + 249;
pub const CURLOPT_PROXY_SSLVERSION: CURLoption = CURLOPTTYPE_VALUES + 250;
pub const CURLOPT_PROXY_SSLCERT: CURLoption = CURLOPTTYPE_OBJECTPOINT + 254;
pub const CURLOPT_PROXY_SSLCERTTYPE: CURLoption = CURLOPTTYPE_OBJECTPOINT + 255;
pub const CURLOPT_PROXY_SSLKEY: CURLoption = CURLOPTTYPE_OBJECTPOINT + 256;
pub const CURLOPT_PROXY_SSLKEYTYPE: CURLoption = CURLOPTTYPE_OBJECTPOINT + 257;
pub const CURLOPT_PROXY_KEYPASSWD: CURLoption = CURLOPTTYPE_OBJECTPOINT + 258;
pub const CURLOPT_PROXY_SSL_CIPHER_LIST: CURLoption = CURLOPTTYPE_OBJECTPOINT + 259;
pub const CURLOPT_PROXY_CRLFILE: CURLoption = CURLOPTTYPE_OBJECTPOINT + 260;
pub const CURLOPT_PROXY_SSL_OPTIONS: CURLoption = CURLOPTTYPE_LONG + 261;
pub const CURLOPT_SUPPRESS_CONNECT_HEADERS: CURLoption = CURLOPTTYPE_LONG + 265;

pub const CURLOPT_ABSTRACT_UNIX_SOCKET: CURLoption = CURLOPTTYPE_OBJECTPOINT + 264;

pub const CURLOPT_DOH_URL: CURLoption = CURLOPTTYPE_OBJECTPOINT + 279;
pub const CURLOPT_UPLOAD_BUFFERSIZE: CURLoption = CURLOPTTYPE_LONG + 280;

pub const CURLOPT_HTTP09_ALLOWED: CURLoption = CURLOPTTYPE_LONG + 285;

pub const CURLOPT_MAXAGE_CONN: CURLoption = CURLOPTTYPE_LONG + 288;

pub const CURLOPT_SSLCERT_BLOB: CURLoption = CURLOPTTYPE_BLOB + 291;
pub const CURLOPT_SSLKEY_BLOB: CURLoption = CURLOPTTYPE_BLOB + 292;
pub const CURLOPT_PROXY_SSLCERT_BLOB: CURLoption = CURLOPTTYPE_BLOB + 293;
pub const CURLOPT_PROXY_SSLKEY_BLOB: CURLoption = CURLOPTTYPE_BLOB + 294;
pub const CURLOPT_ISSUERCERT_BLOB: CURLoption = CURLOPTTYPE_BLOB + 295;

pub const CURLOPT_PROXY_ISSUERCERT: CURLoption = CURLOPTTYPE_OBJECTPOINT + 296;
pub const CURLOPT_PROXY_ISSUERCERT_BLOB: CURLoption = CURLOPTTYPE_BLOB + 297;

pub const CURLOPT_AWS_SIGV4: CURLoption = CURLOPTTYPE_OBJECTPOINT + 305;

pub const CURLOPT_DOH_SSL_VERIFYPEER: CURLoption = CURLOPTTYPE_LONG + 306;
pub const CURLOPT_DOH_SSL_VERIFYHOST: CURLoption = CURLOPTTYPE_LONG + 307;
pub const CURLOPT_DOH_SSL_VERIFYSTATUS: CURLoption = CURLOPTTYPE_LONG + 308;
pub const CURLOPT_CAINFO_BLOB: CURLoption = CURLOPTTYPE_BLOB + 309;
pub const CURLOPT_PROXY_CAINFO_BLOB: CURLoption = CURLOPTTYPE_BLOB + 310;

pub const CURLOPT_HTTPSIG_ALGORITHM: CURLoption = CURLOPTTYPE_VALUES + 329;
pub const CURLOPT_HTTPSIG_KEY: CURLoption = CURLOPTTYPE_OBJECTPOINT + 330;
pub const CURLOPT_HTTPSIG_KEYID: CURLoption = CURLOPTTYPE_OBJECTPOINT + 331;
pub const CURLOPT_HTTPSIG_HEADERS: CURLoption = CURLOPTTYPE_OBJECTPOINT + 332;
pub const CURLOPT_PREREQFUNCTION: CURLoption = CURLOPTTYPE_FUNCTIONPOINT + 312;
pub const CURLOPT_PREREQDATA: CURLoption = CURLOPTTYPE_OBJECTPOINT + 313;

pub const CURLOPT_WS_OPTIONS: CURLoption = CURLOPTTYPE_LONG + 320;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct curl_ws_frame {
    pub age: c_int,
    pub flags: c_int,
    pub offset: curl_off_t,
    pub bytesleft: curl_off_t,
    pub len: size_t,
}

pub const CURLWS_TEXT: c_int = 1 << 0;
pub const CURLWS_BINARY: c_int = 1 << 1;
pub const CURLWS_CONT: c_int = 1 << 2;
pub const CURLWS_CLOSE: c_int = 1 << 3;
pub const CURLWS_PING: c_int = 1 << 4;
pub const CURLWS_OFFSET: c_int = 1 << 5;
pub const CURLWS_PONG: c_int = 1 << 6;

pub const CURLWS_RAW_MODE: c_long = 1 << 0;
pub const CURLWS_NOAUTOPONG: c_long = 1 << 1;

pub const CURL_IPRESOLVE_WHATEVER: c_int = 0;
pub const CURL_IPRESOLVE_V4: c_int = 1;
pub const CURL_IPRESOLVE_V6: c_int = 2;

pub const CURLSSLOPT_ALLOW_BEAST: c_long = 1 << 0;
pub const CURLSSLOPT_NO_REVOKE: c_long = 1 << 1;
pub const CURLSSLOPT_NO_PARTIALCHAIN: c_long = 1 << 2;
pub const CURLSSLOPT_REVOKE_BEST_EFFORT: c_long = 1 << 3;
pub const CURLSSLOPT_NATIVE_CA: c_long = 1 << 4;
pub const CURLSSLOPT_AUTO_CLIENT_CERT: c_long = 1 << 5;

/// These enums are for use with the CURLOPT_HTTP_VERSION option.
///
/// Setting this means we don't care, and that we'd like the library to choose
/// the best possible for us!
pub const CURL_HTTP_VERSION_NONE: c_int = 0;
/// Please use HTTP 1.0 in the request
pub const CURL_HTTP_VERSION_1_0: c_int = 1;
/// Please use HTTP 1.1 in the request
pub const CURL_HTTP_VERSION_1_1: c_int = 2;
/// Please use HTTP 2 in the request
/// (Added in CURL 7.33.0)
pub const CURL_HTTP_VERSION_2_0: c_int = 3;
/// Use version 2 for HTTPS, version 1.1 for HTTP
/// (Added in CURL 7.47.0)
pub const CURL_HTTP_VERSION_2TLS: c_int = 4;
/// Please use HTTP 2 without HTTP/1.1 Upgrade
/// (Added in CURL 7.49.0)
pub const CURL_HTTP_VERSION_2_PRIOR_KNOWLEDGE: c_int = 5;
/// Use HTTP/3, fallback to HTTP/2 or HTTP/1 if needed.
/// (Added in CURL 7.66.0)
pub const CURL_HTTP_VERSION_3: c_int = 30;

// Note that the type here is wrong, it's just intended to just be an enum.
pub const CURL_SSLVERSION_DEFAULT: __enum_ty = 0;
pub const CURL_SSLVERSION_TLSv1: __enum_ty = 1;
pub const CURL_SSLVERSION_SSLv2: __enum_ty = 2;
pub const CURL_SSLVERSION_SSLv3: __enum_ty = 3;
pub const CURL_SSLVERSION_TLSv1_0: __enum_ty = 4;
pub const CURL_SSLVERSION_TLSv1_1: __enum_ty = 5;
pub const CURL_SSLVERSION_TLSv1_2: __enum_ty = 6;
pub const CURL_SSLVERSION_TLSv1_3: __enum_ty = 7;

pub const CURLOPT_READDATA: CURLoption = CURLOPT_INFILE;
pub const CURLOPT_WRITEDATA: CURLoption = CURLOPT_FILE;
pub const CURLOPT_HEADERDATA: CURLoption = CURLOPT_WRITEHEADER;

pub type curl_TimeCond = __enum_ty;
pub const CURL_TIMECOND_NONE: curl_TimeCond = 0;
pub const CURL_TIMECOND_IFMODSINCE: curl_TimeCond = 1;
pub const CURL_TIMECOND_IFUNMODSINCE: curl_TimeCond = 2;
pub const CURL_TIMECOND_LASTMOD: curl_TimeCond = 3;

pub type CURLformoption = __enum_ty;
pub const CURLFORM_NOTHING: CURLformoption = 0;
pub const CURLFORM_COPYNAME: CURLformoption = 1;
pub const CURLFORM_PTRNAME: CURLformoption = 2;
pub const CURLFORM_NAMELENGTH: CURLformoption = 3;
pub const CURLFORM_COPYCONTENTS: CURLformoption = 4;
pub const CURLFORM_PTRCONTENTS: CURLformoption = 5;
pub const CURLFORM_CONTENTSLENGTH: CURLformoption = 6;
pub const CURLFORM_FILECONTENT: CURLformoption = 7;
pub const CURLFORM_ARRAY: CURLformoption = 8;
pub const CURLFORM_OBSOLETE: CURLformoption = 9;
pub const CURLFORM_FILE: CURLformoption = 10;
pub const CURLFORM_BUFFER: CURLformoption = 11;
pub const CURLFORM_BUFFERPTR: CURLformoption = 12;
pub const CURLFORM_BUFFERLENGTH: CURLformoption = 13;
pub const CURLFORM_CONTENTTYPE: CURLformoption = 14;
pub const CURLFORM_CONTENTHEADER: CURLformoption = 15;
pub const CURLFORM_FILENAME: CURLformoption = 16;
pub const CURLFORM_END: CURLformoption = 17;
pub const CURLFORM_STREAM: CURLformoption = 19;

pub type CURLFORMcode = __enum_ty;
pub const CURL_FORMADD_OK: CURLFORMcode = 0;
pub const CURL_FORMADD_MEMORY: CURLFORMcode = 1;
pub const CURL_FORMADD_OPTION_TWICE: CURLFORMcode = 2;
pub const CURL_FORMADD_NULL: CURLFORMcode = 3;
pub const CURL_FORMADD_UNKNOWN_OPTION: CURLFORMcode = 4;
pub const CURL_FORMADD_INCOMPLETE: CURLFORMcode = 5;
pub const CURL_FORMADD_ILLEGAL_ARRAY: CURLFORMcode = 6;
pub const CURL_FORMADD_DISABLED: CURLFORMcode = 7;

pub const CURL_REDIR_POST_301: c_ulong = 1;
pub const CURL_REDIR_POST_302: c_ulong = 2;
pub const CURL_REDIR_POST_303: c_ulong = 4;
pub const CURL_REDIR_POST_ALL: c_ulong =
    CURL_REDIR_POST_301 | CURL_REDIR_POST_302 | CURL_REDIR_POST_303;

#[repr(C)]
pub struct curl_forms {
    pub option: CURLformoption,
    pub value: *const c_char,
}

pub type curl_formget_callback = extern "C" fn(*mut c_void, *const c_char, size_t) -> size_t;

#[repr(C)]
pub struct curl_slist {
    pub data: *mut c_char,
    pub next: *mut curl_slist,
}

#[repr(C)]
pub struct curl_certinfo {
    pub num_of_certs: c_int,
    pub certinfo: *mut *mut curl_slist,
}

// pub type curl_sslbackend = __enum_ty;
// pub const CURLSSLBACKEND_NONE: curl_sslbackend = 0;
// pub const CURLSSLBACKEND_OPENSSL: curl_sslbackend = 1;
// pub const CURLSSLBACKEND_GNUTLS: curl_sslbackend = 2;
// pub const CURLSSLBACKEND_NSS: curl_sslbackend = 3;
// pub const CURLSSLBACKEND_QSOSSL: curl_sslbackend = 4;
// pub const CURLSSLBACKEND_GSKIT: curl_sslbackend = 5;
// pub const CURLSSLBACKEND_POLARSSL: curl_sslbackend = 6;
// pub const CURLSSLBACKEND_CYASSL: curl_sslbackend = 7;
// pub const CURLSSLBACKEND_SCHANNEL: curl_sslbackend = 8;
// pub const CURLSSLBACKEND_DARWINSSL: curl_sslbackend = 9;

// #[repr(C)]
// pub struct curl_tlssessioninfo {
//     pub backend: curl_sslbackend,
//     pub internals: *mut c_void,
// }

pub const CURLINFO_STRING: CURLINFO = 0x100000;
pub const CURLINFO_LONG: CURLINFO = 0x200000;
pub const CURLINFO_DOUBLE: CURLINFO = 0x300000;
pub const CURLINFO_SLIST: CURLINFO = 0x400000;
pub const CURLINFO_SOCKET: CURLINFO = 0x500000;
pub const CURLINFO_MASK: CURLINFO = 0x0fffff;
pub const CURLINFO_TYPEMASK: CURLINFO = 0xf00000;

pub const CURLINFO_EFFECTIVE_URL: CURLINFO = CURLINFO_STRING + 1;
pub const CURLINFO_RESPONSE_CODE: CURLINFO = CURLINFO_LONG + 2;
pub const CURLINFO_TOTAL_TIME: CURLINFO = CURLINFO_DOUBLE + 3;
pub const CURLINFO_NAMELOOKUP_TIME: CURLINFO = CURLINFO_DOUBLE + 4;
pub const CURLINFO_CONNECT_TIME: CURLINFO = CURLINFO_DOUBLE + 5;
pub const CURLINFO_PRETRANSFER_TIME: CURLINFO = CURLINFO_DOUBLE + 6;
pub const CURLINFO_SIZE_UPLOAD: CURLINFO = CURLINFO_DOUBLE + 7;
pub const CURLINFO_SIZE_DOWNLOAD: CURLINFO = CURLINFO_DOUBLE + 8;
pub const CURLINFO_SPEED_DOWNLOAD: CURLINFO = CURLINFO_DOUBLE + 9;
pub const CURLINFO_SPEED_UPLOAD: CURLINFO = CURLINFO_DOUBLE + 10;
pub const CURLINFO_HEADER_SIZE: CURLINFO = CURLINFO_LONG + 11;
pub const CURLINFO_REQUEST_SIZE: CURLINFO = CURLINFO_LONG + 12;
pub const CURLINFO_SSL_VERIFYRESULT: CURLINFO = CURLINFO_LONG + 13;
pub const CURLINFO_FILETIME: CURLINFO = CURLINFO_LONG + 14;
pub const CURLINFO_CONTENT_LENGTH_DOWNLOAD: CURLINFO = CURLINFO_DOUBLE + 15;
pub const CURLINFO_CONTENT_LENGTH_UPLOAD: CURLINFO = CURLINFO_DOUBLE + 16;
pub const CURLINFO_STARTTRANSFER_TIME: CURLINFO = CURLINFO_DOUBLE + 17;
pub const CURLINFO_CONTENT_TYPE: CURLINFO = CURLINFO_STRING + 18;
pub const CURLINFO_REDIRECT_TIME: CURLINFO = CURLINFO_DOUBLE + 19;
pub const CURLINFO_REDIRECT_COUNT: CURLINFO = CURLINFO_LONG + 20;
pub const CURLINFO_PRIVATE: CURLINFO = CURLINFO_STRING + 21;
pub const CURLINFO_HTTP_CONNECTCODE: CURLINFO = CURLINFO_LONG + 22;
pub const CURLINFO_HTTPAUTH_AVAIL: CURLINFO = CURLINFO_LONG + 23;
pub const CURLINFO_PROXYAUTH_AVAIL: CURLINFO = CURLINFO_LONG + 24;
pub const CURLINFO_OS_ERRNO: CURLINFO = CURLINFO_LONG + 25;
pub const CURLINFO_NUM_CONNECTS: CURLINFO = CURLINFO_LONG + 26;
pub const CURLINFO_SSL_ENGINES: CURLINFO = CURLINFO_SLIST + 27;
pub const CURLINFO_COOKIELIST: CURLINFO = CURLINFO_SLIST + 28;
pub const CURLINFO_LASTSOCKET: CURLINFO = CURLINFO_LONG + 29;
pub const CURLINFO_FTP_ENTRY_PATH: CURLINFO = CURLINFO_STRING + 30;
pub const CURLINFO_REDIRECT_URL: CURLINFO = CURLINFO_STRING + 31;
pub const CURLINFO_PRIMARY_IP: CURLINFO = CURLINFO_STRING + 32;
pub const CURLINFO_APPCONNECT_TIME: CURLINFO = CURLINFO_DOUBLE + 33;
pub const CURLINFO_CERTINFO: CURLINFO = CURLINFO_SLIST + 34;
pub const CURLINFO_CONDITION_UNMET: CURLINFO = CURLINFO_LONG + 35;
pub const CURLINFO_RTSP_SESSION_ID: CURLINFO = CURLINFO_STRING + 36;
pub const CURLINFO_RTSP_CLIENT_CSEQ: CURLINFO = CURLINFO_LONG + 37;
pub const CURLINFO_RTSP_SERVER_CSEQ: CURLINFO = CURLINFO_LONG + 38;
pub const CURLINFO_RTSP_CSEQ_RECV: CURLINFO = CURLINFO_LONG + 39;
pub const CURLINFO_PRIMARY_PORT: CURLINFO = CURLINFO_LONG + 40;
pub const CURLINFO_LOCAL_IP: CURLINFO = CURLINFO_STRING + 41;
pub const CURLINFO_LOCAL_PORT: CURLINFO = CURLINFO_LONG + 42;
pub const CURLINFO_ACTIVESOCKET: CURLINFO = CURLINFO_SOCKET + 44;
// pub const CURLINFO_TLS_SESSION: CURLINFO = CURLINFO_SLIST + 43;

pub type curl_closepolicy = __enum_ty;
pub const CURLCLOSEPOLICY_NONE: curl_closepolicy = 0;
pub const CURLCLOSEPOLICY_OLDEST: curl_closepolicy = 1;
pub const CURLCLOSEPOLICY_LEAST_RECENTLY_USED: curl_closepolicy = 2;
pub const CURLCLOSEPOLICY_LEAST_TRAFFIC: curl_closepolicy = 3;
pub const CURLCLOSEPOLICY_SLOWEST: curl_closepolicy = 4;
pub const CURLCLOSEPOLICY_CALLBACK: curl_closepolicy = 5;

pub const CURL_GLOBAL_SSL: c_long = 1 << 0;
pub const CURL_GLOBAL_WIN32: c_long = 1 << 1;
pub const CURL_GLOBAL_ALL: c_long = CURL_GLOBAL_SSL | CURL_GLOBAL_WIN32;
pub const CURL_GLOBAL_NOTHING: c_long = 0;
pub const CURL_GLOBAL_DEFAULT: c_long = CURL_GLOBAL_ALL;
// pub const CURL_GLOBAL_ACK_EINTR: c_long = 1 << 2;

pub type curl_lock_data = __enum_ty;
pub const CURL_LOCK_DATA_NONE: curl_lock_data = 0;
pub const CURL_LOCK_DATA_SHARE: curl_lock_data = 1;
pub const CURL_LOCK_DATA_COOKIE: curl_lock_data = 2;
pub const CURL_LOCK_DATA_DNS: curl_lock_data = 3;
pub const CURL_LOCK_DATA_SSL_SESSION: curl_lock_data = 4;
pub const CURL_LOCK_DATA_CONNECT: curl_lock_data = 5;

pub type curl_lock_access = __enum_ty;
pub const CURL_LOCK_ACCESS_NONE: curl_lock_access = 0;
pub const CURL_LOCK_ACCESS_SHARED: curl_lock_access = 1;
pub const CURL_LOCK_ACCESS_SINGLE: curl_lock_access = 2;

pub type curl_lock_function =
    extern "C" fn(*mut CURL, curl_lock_data, curl_lock_access, *mut c_void);
pub type curl_unlock_function = extern "C" fn(*mut CURL, curl_lock_data, *mut c_void);

pub enum CURLSH {}

pub type CURLSHcode = __enum_ty;
pub const CURLSHE_OK: CURLSHcode = 0;
pub const CURLSHE_BAD_OPTION: CURLSHcode = 1;
pub const CURLSHE_IN_USE: CURLSHcode = 2;
pub const CURLSHE_INVALID: CURLSHcode = 3;
pub const CURLSHE_NOMEM: CURLSHcode = 4;
// pub const CURLSHE_NOT_BUILT_IN: CURLSHcode = 5;

pub type CURLSHoption = __enum_ty;
pub const CURLSHOPT_NONE: CURLSHoption = 0;
pub const CURLSHOPT_SHARE: CURLSHoption = 1;
pub const CURLSHOPT_UNSHARE: CURLSHoption = 2;
pub const CURLSHOPT_LOCKFUNC: CURLSHoption = 3;
pub const CURLSHOPT_UNLOCKFUNC: CURLSHoption = 4;
pub const CURLSHOPT_USERDATA: CURLSHoption = 5;

pub const CURLVERSION_FIRST: CURLversion = 0;
pub const CURLVERSION_SECOND: CURLversion = 1;
pub const CURLVERSION_THIRD: CURLversion = 2;
pub const CURLVERSION_FOURTH: CURLversion = 3;
pub const CURLVERSION_FIFTH: CURLversion = 4;
pub const CURLVERSION_SIXTH: CURLversion = 5;
pub const CURLVERSION_SEVENTH: CURLversion = 6;
pub const CURLVERSION_EIGHTH: CURLversion = 7;
pub const CURLVERSION_NINTH: CURLversion = 8;
pub const CURLVERSION_TENTH: CURLversion = 9;
pub const CURLVERSION_ELEVENTH: CURLversion = 10;
pub const CURLVERSION_TWELFTH: CURLversion = 11;
pub const CURLVERSION_NOW: CURLversion = CURLVERSION_TWELFTH;

#[repr(C)]
pub struct curl_version_info_data {
    pub age: CURLversion,
    pub version: *const c_char,
    pub version_num: c_uint,
    pub host: *const c_char,
    pub features: c_int,
    pub ssl_version: *const c_char,
    pub ssl_version_num: c_long,
    pub libz_version: *const c_char,
    pub protocols: *const *const c_char,
    pub ares: *const c_char,
    pub ares_num: c_int,
    pub libidn: *const c_char,
    pub iconv_ver_num: c_int,
    pub libssh_version: *const c_char,
    pub brotli_ver_num: c_uint,
    pub brotli_version: *const c_char,
    pub nghttp2_ver_num: c_uint,
    pub nghttp2_version: *const c_char,
    pub quic_version: *const c_char,
    pub cainfo: *const c_char,
    pub capath: *const c_char,
    pub zstd_ver_num: c_uint,
    pub zstd_version: *const c_char,
    pub hyper_version: *const c_char,
    pub gsasl_version: *const c_char,
    pub feature_names: *const *const c_char,
    pub rtmp_version: *const c_char,
}

pub const CURL_VERSION_IPV6: c_int = 1 << 0;
pub const CURL_VERSION_KERBEROS4: c_int = 1 << 1;
pub const CURL_VERSION_SSL: c_int = 1 << 2;
pub const CURL_VERSION_LIBZ: c_int = 1 << 3;
pub const CURL_VERSION_NTLM: c_int = 1 << 4;
pub const CURL_VERSION_GSSNEGOTIATE: c_int = 1 << 5;
pub const CURL_VERSION_DEBUG: c_int = 1 << 6;
pub const CURL_VERSION_ASYNCHDNS: c_int = 1 << 7;
pub const CURL_VERSION_SPNEGO: c_int = 1 << 8;
pub const CURL_VERSION_LARGEFILE: c_int = 1 << 9;
pub const CURL_VERSION_IDN: c_int = 1 << 10;
pub const CURL_VERSION_SSPI: c_int = 1 << 11;
pub const CURL_VERSION_CONV: c_int = 1 << 12;
pub const CURL_VERSION_CURLDEBUG: c_int = 1 << 13;
pub const CURL_VERSION_TLSAUTH_SRP: c_int = 1 << 14;
pub const CURL_VERSION_NTLM_WB: c_int = 1 << 15;
pub const CURL_VERSION_HTTP2: c_int = 1 << 16;
pub const CURL_VERSION_UNIX_SOCKETS: c_int = 1 << 19;
pub const CURL_VERSION_HTTPS_PROXY: c_int = 1 << 21;
pub const CURL_VERSION_BROTLI: c_int = 1 << 23;
pub const CURL_VERSION_ALTSVC: c_int = 1 << 24;
pub const CURL_VERSION_HTTP3: c_int = 1 << 25;
pub const CURL_VERSION_ZSTD: c_int = 1 << 26;
pub const CURL_VERSION_UNICODE: c_int = 1 << 27;
pub const CURL_VERSION_HSTS: c_int = 1 << 28;
pub const CURL_VERSION_GSASL: c_int = 1 << 29;

pub const CURLPAUSE_RECV: c_int = 1 << 0;
pub const CURLPAUSE_RECV_CONT: c_int = 0;
pub const CURLPAUSE_SEND: c_int = 1 << 2;
pub const CURLPAUSE_SEND_CONT: c_int = 0;

pub enum CURLM {}

pub type CURLMcode = c_int;
pub const CURLM_CALL_MULTI_PERFORM: CURLMcode = -1;
pub const CURLM_OK: CURLMcode = 0;
pub const CURLM_BAD_HANDLE: CURLMcode = 1;
pub const CURLM_BAD_EASY_HANDLE: CURLMcode = 2;
pub const CURLM_OUT_OF_MEMORY: CURLMcode = 3;
pub const CURLM_INTERNAL_ERROR: CURLMcode = 4;
pub const CURLM_BAD_SOCKET: CURLMcode = 5;
pub const CURLM_UNKNOWN_OPTION: CURLMcode = 6;
// pub const CURLM_ADDED_ALREADY: CURLMcode = 7;

pub type CURLMSG = __enum_ty;
pub const CURLMSG_NONE: CURLMSG = 0;
pub const CURLMSG_DONE: CURLMSG = 1;

#[repr(C)]
pub struct CURLMsg {
    pub msg: CURLMSG,
    pub easy_handle: *mut CURL,
    pub data: *mut c_void,
}

pub const CURL_WAIT_POLLIN: c_short = 0x1;
pub const CURL_WAIT_POLLPRI: c_short = 0x2;
pub const CURL_WAIT_POLLOUT: c_short = 0x4;

#[repr(C)]
pub struct curl_waitfd {
    pub fd: curl_socket_t,
    pub events: c_short,
    pub revents: c_short,
}

pub const CURL_POLL_NONE: c_int = 0;
pub const CURL_POLL_IN: c_int = 1;
pub const CURL_POLL_OUT: c_int = 2;
pub const CURL_POLL_INOUT: c_int = 3;
pub const CURL_POLL_REMOVE: c_int = 4;
pub const CURL_CSELECT_IN: c_int = 1;
pub const CURL_CSELECT_OUT: c_int = 2;
pub const CURL_CSELECT_ERR: c_int = 4;
pub const CURL_SOCKET_TIMEOUT: curl_socket_t = CURL_SOCKET_BAD;

pub type curl_socket_callback =
    extern "C" fn(*mut CURL, curl_socket_t, c_int, *mut c_void, *mut c_void) -> c_int;
pub type curl_multi_timer_callback = extern "C" fn(*mut CURLM, c_long, *mut c_void) -> c_int;

pub type CURLMoption = __enum_ty;
pub const CURLMOPT_SOCKETFUNCTION: CURLMoption = CURLOPTTYPE_FUNCTIONPOINT + 1;
pub const CURLMOPT_SOCKETDATA: CURLMoption = CURLOPTTYPE_OBJECTPOINT + 2;
pub const CURLMOPT_PIPELINING: CURLMoption = CURLOPTTYPE_LONG + 3;
pub const CURLMOPT_TIMERFUNCTION: CURLMoption = CURLOPTTYPE_FUNCTIONPOINT + 4;
pub const CURLMOPT_TIMERDATA: CURLMoption = CURLOPTTYPE_OBJECTPOINT + 5;
pub const CURLMOPT_MAXCONNECTS: CURLMoption = CURLOPTTYPE_LONG + 6;
pub const CURLMOPT_MAX_HOST_CONNECTIONS: CURLMoption = CURLOPTTYPE_LONG + 7;
pub const CURLMOPT_MAX_PIPELINE_LENGTH: CURLMoption = CURLOPTTYPE_LONG + 8;
pub const CURLMOPT_CONTENT_LENGTH_PENALTY_SIZE: CURLMoption = CURLOPTTYPE_OFF_T + 9;
pub const CURLMOPT_CHUNK_LENGTH_PENALTY_SIZE: CURLMoption = CURLOPTTYPE_OFF_T + 10;
pub const CURLMOPT_PIPELINING_SITE_BL: CURLMoption = CURLOPTTYPE_OBJECTPOINT + 11;
pub const CURLMOPT_PIPELINING_SERVER_BL: CURLMoption = CURLOPTTYPE_OBJECTPOINT + 12;
pub const CURLMOPT_MAX_TOTAL_CONNECTIONS: CURLMoption = CURLOPTTYPE_LONG + 13;
pub const CURLMOPT_PUSHFUNCTION: CURLMoption = CURLOPTTYPE_FUNCTIONPOINT + 14;
pub const CURLMOPT_PUSHDATA: CURLMoption = CURLOPTTYPE_OBJECTPOINT + 15;
pub const CURLMOPT_MAX_CONCURRENT_STREAMS: CURLMoption = CURLOPTTYPE_LONG + 16;

// These enums are for use with the CURLMOPT_PIPELINING option.
pub const CURLPIPE_NOTHING: c_long = 0;
pub const CURLPIPE_HTTP1: c_long = 1;
pub const CURLPIPE_MULTIPLEX: c_long = 2;

pub const CURL_ERROR_SIZE: usize = 256;

pub type curl_opensocket_callback =
    extern "C" fn(*mut c_void, curlsocktype, *mut curl_sockaddr) -> curl_socket_t;
pub type curlsocktype = __enum_ty;
pub const CURLSOCKTYPE_IPCXN: curlsocktype = 0;
pub const CURLSOCKTYPE_ACCEPT: curlsocktype = 1;
pub const CURLSOCKTYPE_LAST: curlsocktype = 2;

#[repr(C)]
pub struct curl_sockaddr {
    pub family: c_int,
    pub socktype: c_int,
    pub protocol: c_int,
    pub addrlen: c_uint,
    #[cfg(unix)]
    pub addr: libc::sockaddr,
    #[cfg(windows)]
    pub addr: SOCKADDR,
}

extern "C" {
    pub fn curl_formadd(
        httppost: *mut *mut curl_httppost,
        last_post: *mut *mut curl_httppost,
        ...
    ) -> CURLFORMcode;
    pub fn curl_formget(
        form: *mut curl_httppost,
        arg: *mut c_void,
        append: curl_formget_callback,
    ) -> c_int;
    pub fn curl_formfree(form: *mut curl_httppost);

    pub fn curl_version() -> *mut c_char;

    pub fn curl_easy_escape(handle: *mut CURL, string: *const c_char, length: c_int)
        -> *mut c_char;
    pub fn curl_easy_unescape(
        handle: *mut CURL,
        string: *const c_char,
        length: c_int,
        outlength: *mut c_int,
    ) -> *mut c_char;
    pub fn curl_free(p: *mut c_void);

    pub fn curl_global_init(flags: c_long) -> CURLcode;
    pub fn curl_global_init_mem(
        flags: c_long,
        m: curl_malloc_callback,
        f: curl_free_callback,
        r: curl_realloc_callback,
        s: curl_strdup_callback,
        c: curl_calloc_callback,
    ) -> CURLcode;
    pub fn curl_global_cleanup();

    pub fn curl_slist_append(list: *mut curl_slist, val: *const c_char) -> *mut curl_slist;
    pub fn curl_slist_free_all(list: *mut curl_slist);

    pub fn curl_getdate(p: *const c_char, _: *const time_t) -> time_t;

    pub fn curl_share_init() -> *mut CURLSH;
    pub fn curl_share_setopt(sh: *mut CURLSH, opt: CURLSHoption, ...) -> CURLSHcode;
    pub fn curl_share_cleanup(sh: *mut CURLSH) -> CURLSHcode;

    pub fn curl_version_info(t: CURLversion) -> *mut curl_version_info_data;

    pub fn curl_easy_strerror(code: CURLcode) -> *const c_char;
    pub fn curl_share_strerror(code: CURLSHcode) -> *const c_char;
    pub fn curl_easy_pause(handle: *mut CURL, bitmask: c_int) -> CURLcode;

    pub fn curl_easy_init() -> *mut CURL;
    pub fn curl_easy_setopt(curl: *mut CURL, option: CURLoption, ...) -> CURLcode;
    pub fn curl_easy_perform(curl: *mut CURL) -> CURLcode;
    pub fn curl_easy_cleanup(curl: *mut CURL);
    pub fn curl_easy_getinfo(curl: *mut CURL, info: CURLINFO, ...) -> CURLcode;
    pub fn curl_easy_moli_timing_init(
        curl: *mut CURL,
        clock: Option<extern "C" fn() -> curl_off_t>,
        dns_start: curl_off_t,
        dns_end: curl_off_t,
        has_dns: c_int,
    ) -> CURLcode;
    pub fn curl_easy_moli_connection_timing(
        curl: *mut CURL,
        timing: *mut curl_moli_connection_timing,
    ) -> CURLcode;
    pub fn curl_easy_duphandle(curl: *mut CURL) -> *mut CURL;
    pub fn curl_easy_reset(curl: *mut CURL);
    pub fn curl_easy_recv(
        curl: *mut CURL,
        buffer: *mut c_void,
        buflen: size_t,
        n: *mut size_t,
    ) -> CURLcode;
    pub fn curl_easy_send(
        curl: *mut CURL,
        buffer: *const c_void,
        buflen: size_t,
        n: *mut size_t,
    ) -> CURLcode;

    #[cfg(feature = "websocket")]
    pub fn curl_ws_recv(
        curl: *mut CURL,
        buffer: *mut c_void,
        buflen: size_t,
        recv: *mut size_t,
        meta: *mut *const curl_ws_frame,
    ) -> CURLcode;
    #[cfg(feature = "websocket")]
    pub fn curl_ws_send(
        curl: *mut CURL,
        buffer: *const c_void,
        buflen: size_t,
        sent: *mut size_t,
        fragsize: curl_off_t,
        flags: c_uint,
    ) -> CURLcode;
    #[cfg(feature = "websocket")]
    pub fn curl_ws_meta(curl: *mut CURL) -> *const curl_ws_frame;
    #[cfg(feature = "ws_start_frame_8_16_0")]
    pub fn curl_ws_start_frame(curl: *mut CURL, flags: c_uint, frame_len: curl_off_t) -> CURLcode;

    #[cfg(feature = "upkeep_7_62_0")]
    pub fn curl_easy_upkeep(curl: *mut CURL) -> CURLcode;

    pub fn curl_multi_init() -> *mut CURLM;
    pub fn curl_multi_add_handle(multi_handle: *mut CURLM, curl_handle: *mut CURL) -> CURLMcode;
    pub fn curl_multi_remove_handle(multi_handle: *mut CURLM, curl_handle: *mut CURL) -> CURLMcode;
    pub fn curl_multi_fdset(
        multi_handle: *mut CURLM,
        read_fd_set: *mut fd_set,
        write_fd_set: *mut fd_set,
        exc_fd_set: *mut fd_set,
        max_fd: *mut c_int,
    ) -> CURLMcode;
    pub fn curl_multi_wait(
        multi_handle: *mut CURLM,
        extra_fds: *mut curl_waitfd,
        extra_nfds: c_uint,
        timeout_ms: c_int,
        ret: *mut c_int,
    ) -> CURLMcode;

    #[cfg(feature = "poll_7_68_0")]
    pub fn curl_multi_poll(
        multi_handle: *mut CURLM,
        extra_fds: *mut curl_waitfd,
        extra_nfds: c_uint,
        timeout_ms: c_int,
        ret: *mut c_int,
    ) -> CURLMcode;

    #[cfg(feature = "poll_7_68_0")]
    pub fn curl_multi_wakeup(multi_handle: *mut CURLM) -> CURLMcode;

    pub fn curl_multi_perform(multi_handle: *mut CURLM, running_handles: *mut c_int) -> CURLMcode;
    pub fn curl_multi_cleanup(multi_handle: *mut CURLM) -> CURLMcode;
    pub fn curl_multi_info_read(
        multi_handle: *mut CURLM,
        msgs_in_queue: *mut c_int,
    ) -> *mut CURLMsg;
    pub fn curl_multi_strerror(code: CURLMcode) -> *const c_char;
    pub fn curl_multi_socket(
        multi_handle: *mut CURLM,
        s: curl_socket_t,
        running_handles: *mut c_int,
    ) -> CURLMcode;
    pub fn curl_multi_socket_action(
        multi_handle: *mut CURLM,
        s: curl_socket_t,
        ev_bitmask: c_int,
        running_handles: *mut c_int,
    ) -> CURLMcode;
    pub fn curl_multi_socket_all(
        multi_handle: *mut CURLM,
        running_handles: *mut c_int,
    ) -> CURLMcode;
    pub fn curl_multi_timeout(multi_handle: *mut CURLM, milliseconds: *mut c_long) -> CURLMcode;
    pub fn curl_multi_setopt(multi_handle: *mut CURLM, option: CURLMoption, ...) -> CURLMcode;
    pub fn curl_multi_assign(
        multi_handle: *mut CURLM,
        sockfd: curl_socket_t,
        sockp: *mut c_void,
    ) -> CURLMcode;
}

pub fn rust_crate_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[doc(hidden)]
pub fn vendored() -> bool {
    cfg!(libcurl_vendored)
}
