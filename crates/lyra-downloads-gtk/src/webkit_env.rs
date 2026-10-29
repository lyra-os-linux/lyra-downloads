//! Contorno do crash do WebKitGTK 2.52 ao encerrar (lyra-downloads#5,
//! vega#166).
//!
//! O renderer DMA-BUF do WebKit guarda o dispositivo GBM num singleton
//! estático (`WebCore::GBMDevice`). No `exit()` do WebKitWebProcess, o
//! destrutor dele corre contra o desmonte do Mesa e do glvnd e corrompe o
//! heap ("corrupted double-linked list"), deixando um coredump. Com o
//! renderer desligado, o compositing acelerado continua ligado. Retirar
//! quando o WebKit corrigir.

use gio::prelude::*;
use std::ffi::OsStr;
use std::sync::atomic::{AtomicBool, Ordering};

pub const DISABLE_DMABUF: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";

/// Se a variável foi definida pelo Lyra Downloads (e não herdada do usuário).
static SET_BY_APP: AtomicBool = AtomicBool::new(false);

/// Desliga o renderer DMA-BUF do WebKit. Chamar no início do `main`, antes
/// de qualquer thread. Um valor já definido pelo usuário é respeitado.
pub fn apply() {
    if should_set(std::env::var_os(DISABLE_DMABUF).as_deref()) {
        // SAFETY: chamado no início do main, com uma única thread.
        unsafe { std::env::set_var(DISABLE_DMABUF, "1") };
        SET_BY_APP.store(true, Ordering::Relaxed);
    }
}

fn should_set(existing: Option<&OsStr>) -> bool {
    existing.is_none()
}

/// Variáveis que o backend iniciado pela janela não deve herdar: ele abre
/// pastas e o aria2c, que não precisam do contorno.
pub fn child_unset() -> &'static [&'static str] {
    unset_for(SET_BY_APP.load(Ordering::Relaxed))
}

fn unset_for(set_by_app: bool) -> &'static [&'static str] {
    if set_by_app {
        &[DISABLE_DMABUF]
    } else {
        &[]
    }
}

/// Apps abertos pelo GIO (arquivo ou pasta) também não herdam o contorno.
pub fn launch_context() -> gio::AppLaunchContext {
    let context = gio::AppLaunchContext::new();
    if SET_BY_APP.load(Ordering::Relaxed) {
        context.unsetenv(DISABLE_DMABUF);
    }
    context
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_only_when_the_user_did_not() {
        assert!(should_set(None));
        assert!(!should_set(Some(OsStr::new("0"))));
        assert!(!should_set(Some(OsStr::new("1"))));
    }

    #[test]
    fn backend_loses_only_the_workaround_set_by_the_app() {
        assert_eq!(unset_for(true), &[DISABLE_DMABUF]);
        assert!(unset_for(false).is_empty());
    }
}
