//! Test Report System

pub mod json;
pub mod console;
pub mod html;
pub mod detail_log;

pub use json::JsonReporter;
pub use console::ConsoleReporter;
pub use html::HtmlReporter;
pub use detail_log::write_detail_log;
