//! Test Report System

pub mod console;
pub mod detail_log;
pub mod html;
pub mod json;

pub use console::ConsoleReporter;
pub use detail_log::write_detail_log;
pub use html::HtmlReporter;
pub use json::JsonReporter;
