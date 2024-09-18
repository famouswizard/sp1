#[cfg(target_os = "zkvm")]
use core::arch::asm;

/// Uint256 multiplication operation.
///
/// The result is written over the first input.
///
/// ### Safety
///
/// The caller must ensure that `x` and `y` are valid pointers to data that is aligned along a four
/// byte boundary.
#[allow(unused_variables)]
#[no_mangle]
pub extern "C" fn syscall_uma_mul(x: *mut [u32; 8], y: *const [u32; 8]) {
    #[cfg(target_os = "zkvm")]
    unsafe {
        asm!(
            "ecall",
            in("t0") crate::syscalls::UMA,
            in("a0") x,
            in("a1") y,
        );
    }

    #[cfg(not(target_os = "zkvm"))]
    unreachable!()
}