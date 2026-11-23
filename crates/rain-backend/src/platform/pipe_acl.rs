//! Who may connect to the pipe: the current Windows user and the system, nobody else.
//! Foundation section 3.2. Copied from the spike `types-stream` (`winsec.rs`), without the parts
//! that only its proof program used.
//!
//! The backend has no log-in of its own. The access list of the pipe is the whole protection, so
//! it is built here, in one place, and a test reads it back from a live pipe.

use std::ffi::c_void;
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr::{addr_of, null_mut};

use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use windows_sys::Win32::Foundation::{HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertSecurityDescriptorToStringSecurityDescriptorW, ConvertSidToStringSidW,
    ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo, SDDL_REVISION_1,
    SE_KERNEL_OBJECT,
};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, ACL_SIZE_INFORMATION, AclSizeInformation,
    DACL_SECURITY_INFORMATION, GetAce, GetAclInformation, GetTokenInformation, SECURITY_ATTRIBUTES,
    TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// The `AceType` of an "access allowed" entry.
const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
/// The `AceType` of an "access denied" entry.
const ACCESS_DENIED_ACE_TYPE: u8 = 1;

fn last_error() -> io::Error {
    io::Error::last_os_error()
}

/// Copies a NUL-terminated UTF-16 string that Windows allocated.
///
/// # Safety
/// `text` must point to a valid, NUL-terminated UTF-16 string that stays alive during the call.
unsafe fn wide_to_string(text: *const u16) -> String {
    let mut len = 0;
    // SAFETY: the caller promises a NUL terminator, so the loop stops before the end of the
    // allocation, and every position before the terminator is readable.
    while unsafe { *text.add(len) } != 0 {
        len += 1;
    }
    // SAFETY: `len` readable `u16` values start at `text`, as just measured.
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, len) })
}

/// Turns a SID into its text form `S-1-5-...`.
///
/// # Safety
/// `sid` must point to a valid SID that stays alive during the call.
unsafe fn sid_to_string(sid: *mut c_void) -> io::Result<String> {
    let mut text: *mut u16 = null_mut();
    // SAFETY: `sid` is valid (caller's promise) and `text` is a valid place to put the pointer.
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return Err(last_error());
    }
    // SAFETY: on success Windows returned a NUL-terminated string that we now own.
    let result = unsafe { wide_to_string(text) };
    // SAFETY: the string was allocated by Windows for us, and the docs say `LocalFree` frees it.
    unsafe { LocalFree(text.cast()) };
    Ok(result)
}

/// The SID of the user this process runs as, as text.
///
/// A SID identifies a machine account. It may be shown to the program that needs it, but never
/// written to a log, a document or a test file.
pub fn current_user_sid_string() -> io::Result<String> {
    let mut token: HANDLE = null_mut();
    // SAFETY: `GetCurrentProcess` returns a pseudo handle that needs no closing, and `token` is
    // a valid place for the new handle.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(last_error());
    }
    // SAFETY: `OpenProcessToken` succeeded, so `token` is a handle that we own; the `OwnedHandle`
    // closes it when this function ends.
    let _token_guard = unsafe { OwnedHandle::from_raw_handle(token.cast()) };

    // The first call only asks how big the buffer must be (it "fails" with the answer).
    let mut len = 0u32;
    // SAFETY: a null buffer of length 0 is the documented way to ask for the size.
    unsafe { GetTokenInformation(token, TokenUser, null_mut(), 0, &mut len) };
    // A `Vec<u64>` is 8-byte aligned, as `TOKEN_USER` (it holds a pointer) requires.
    let mut buffer = vec![0u64; (len as usize).div_ceil(8)];
    // SAFETY: the buffer is at least `len` bytes, as Windows asked.
    let ok =
        unsafe { GetTokenInformation(token, TokenUser, buffer.as_mut_ptr().cast(), len, &mut len) };
    if ok == 0 {
        return Err(last_error());
    }
    // SAFETY: the buffer now holds a `TOKEN_USER` at its start, correctly aligned, and its SID
    // pointer points into the same buffer, which lives until the end of this function.
    unsafe {
        let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
        sid_to_string(user.User.Sid)
    }
}

/// The security descriptor that every instance of the pipe gets.
pub struct PipeSecurity {
    /// A descriptor that Windows allocated; freed in `Drop`.
    descriptor: *mut c_void,
    attributes: SECURITY_ATTRIBUTES,
}

// SAFETY: the descriptor is never changed after it was created, and Windows allows it to be read
// from any thread, so sharing it between threads is sound.
unsafe impl Send for PipeSecurity {}
// SAFETY: see `Send`.
unsafe impl Sync for PipeSecurity {}

impl PipeSecurity {
    /// Only the current user and the system can connect.
    ///
    /// ```text
    /// D:P            a DACL (access list) that is "protected": it inherits nothing
    /// (A;;GA;;;SID)  allow everything to this user's SID
    /// (A;;GA;;;SY)   allow everything to the system, so Windows can still manage the pipe
    /// ```
    /// Everyone else (other users, administrators, "Everyone", anonymous) matches no entry and is
    /// refused. The user's explicit SID is used and not "owner", because the owner of a new
    /// object can be the Administrators group when the process runs elevated.
    pub fn current_user_only() -> io::Result<Self> {
        let sid = current_user_sid_string()?;
        let sddl: Vec<u16> = format!("D:P(A;;GA;;;{sid})(A;;GA;;;SY)")
            .encode_utf16()
            .chain([0])
            .collect();
        let mut descriptor: *mut c_void = null_mut();
        // SAFETY: `sddl` is NUL-terminated and outlives the call; `descriptor` is a valid place
        // for the result; the size output is not wanted and may be null.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                null_mut(),
            )
        };
        if ok == 0 {
            return Err(last_error());
        }
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        Ok(Self {
            descriptor,
            attributes,
        })
    }

    /// Creates one instance of the pipe.
    ///
    /// `first = true` sets `FILE_FLAG_FIRST_PIPE_INSTANCE`: the call fails with "access denied"
    /// (os error 5) when the name already exists. That is the single-instance check, and it also
    /// stops another program from creating the name first and posing as the backend.
    /// Every instance refuses clients from other machines.
    pub fn create(&self, name: &str, first: bool) -> io::Result<NamedPipeServer> {
        let mut options = ServerOptions::new();
        options
            .first_pipe_instance(first)
            .reject_remote_clients(true);
        // SAFETY: `attributes` points at a valid security descriptor that lives as long as `self`,
        // which outlives this call, and Windows copies what it needs during the call.
        unsafe {
            options.create_with_security_attributes_raw(
                rain_types::env::pipe_path(name),
                addr_of!(self.attributes).cast_mut().cast(), // Windows only reads it
            )
        }
    }
}

impl Drop for PipeSecurity {
    fn drop(&mut self) {
        // SAFETY: the descriptor was allocated by Windows for us (see `current_user_only`), is
        // freed only here, and nothing uses it after the struct is gone.
        unsafe { LocalFree(self.descriptor) };
    }
}

/// One entry of an access list, as read back from the system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AceReport {
    /// True for an "allow" entry.
    pub allows: bool,
    /// The access rights. Windows writes `GA` (all) as `0x001F01FF`.
    pub mask: u32,
    /// Who the entry is about, in text form.
    pub sid: String,
}

/// The access list of a pipe, as the system has it (not as we wrote it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AclReport {
    /// The list in SDDL text. It starts with `D:P` when it is protected.
    pub sddl: String,
    pub entries: Vec<AceReport>,
}

/// Reads the access list back from a live pipe handle, a server instance or a client connection.
pub fn read_back_acl(pipe: &impl AsRawHandle) -> io::Result<AclReport> {
    let handle: HANDLE = pipe.as_raw_handle().cast();
    let mut dacl: *mut ACL = null_mut();
    let mut descriptor: *mut c_void = null_mut();
    // SAFETY: `handle` is open for the whole call (it is borrowed from `pipe`); the output places
    // are valid; the owner, group and audit outputs are not wanted and may be null.
    let status = unsafe {
        GetSecurityInfo(
            handle,
            SE_KERNEL_OBJECT,
            DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            &mut dacl,
            null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    // `descriptor` is a block that Windows allocated and that `dacl` points into. It is freed on
    // every path by this guard.
    let _descriptor_guard = LocalMemory(descriptor);

    Ok(AclReport {
        // SAFETY: `descriptor` is the valid descriptor that `GetSecurityInfo` just returned.
        sddl: unsafe { descriptor_to_sddl(descriptor)? },
        // SAFETY: `dacl` is null or points into that descriptor, which stays alive until the
        // guard drops.
        entries: unsafe { read_entries(dacl)? },
    })
}

/// Frees a block that Windows allocated with `LocalAlloc`.
struct LocalMemory(*mut c_void);

impl Drop for LocalMemory {
    fn drop(&mut self) {
        // SAFETY: the pointer is a block from Windows' allocator, freed here and only here.
        unsafe { LocalFree(self.0) };
    }
}

/// # Safety
/// `descriptor` must be a valid security descriptor that stays alive during the call.
unsafe fn descriptor_to_sddl(descriptor: *mut c_void) -> io::Result<String> {
    let mut text: *mut u16 = null_mut();
    // SAFETY: the descriptor is valid (caller's promise); `text` is a valid place for the result.
    let ok = unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(
            descriptor,
            SDDL_REVISION_1,
            DACL_SECURITY_INFORMATION,
            &mut text,
            null_mut(),
        )
    };
    if ok == 0 {
        return Err(last_error());
    }
    let _text_guard = LocalMemory(text.cast());
    // SAFETY: on success `text` is a NUL-terminated string that Windows allocated; the guard
    // frees it after this read.
    Ok(unsafe { wide_to_string(text) })
}

/// # Safety
/// `dacl` must be null or point to a valid access list that stays alive during the call.
unsafe fn read_entries(dacl: *mut ACL) -> io::Result<Vec<AceReport>> {
    // A null list means "everyone may do everything". It has no entries to read.
    if dacl.is_null() {
        return Ok(Vec::new());
    }
    let mut info = ACL_SIZE_INFORMATION {
        AceCount: 0,
        AclBytesInUse: 0,
        AclBytesFree: 0,
    };
    // SAFETY: `info` is a valid, writable place of exactly the size we state.
    let ok = unsafe {
        GetAclInformation(
            dacl,
            (&raw mut info).cast(),
            size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    };
    if ok == 0 {
        return Err(last_error());
    }

    let mut entries = Vec::new();
    for index in 0..info.AceCount {
        let mut ace: *mut c_void = null_mut();
        // SAFETY: `index` is below the entry count that Windows just gave; `ace` is a valid place.
        if unsafe { GetAce(dacl, index, &mut ace) } == 0 {
            return Err(last_error());
        }
        // SAFETY: `ace` points to an entry of the list, and every entry starts with an `ACE_HEADER`.
        let ace_type = unsafe { (*ace.cast::<ACE_HEADER>()).AceType };
        if ace_type != ACCESS_ALLOWED_ACE_TYPE && ace_type != ACCESS_DENIED_ACE_TYPE {
            // Other kinds (audit, object entries) have another layout: report them without a SID.
            entries.push(AceReport {
                allows: false,
                mask: 0,
                sid: String::new(),
            });
            continue;
        }
        // SAFETY: "allow" and "deny" entries have the same layout (header, access mask, then the
        // SID), so reading both as `ACCESS_ALLOWED_ACE` is sound.
        let entry = unsafe { &*ace.cast::<ACCESS_ALLOWED_ACE>() };
        // SAFETY: the SID starts at `SidStart`, inside the entry that is alive.
        let sid = unsafe { sid_to_string(addr_of!(entry.SidStart).cast_mut().cast())? };
        entries.push(AceReport {
            allows: ace_type == ACCESS_ALLOWED_ACE_TYPE,
            mask: entry.Mask,
            sid,
        });
    }
    Ok(entries)
}
