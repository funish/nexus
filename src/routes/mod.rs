//! Route handlers, organized so the module tree mirrors the URL tree:
//! `routes::cdn::*` → `/cdn/**`, `routes::api::winget::*` → `/api/winget/**`.

pub mod api;
pub mod cdn;
