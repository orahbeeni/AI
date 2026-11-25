//! Organise core: the document model, its plain-text storage, file links and the machinery that keeps
//! links pointing at files after they move.

pub mod backup;
pub mod engine;
pub mod links;
pub mod markdown;
pub mod model;
pub mod scan;
pub mod store;
pub mod tracker;
