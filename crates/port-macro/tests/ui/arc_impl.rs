use std::sync::Arc;

#[uima_port_macro::has_port]
pub trait Clock {}

struct System;

impl Clock for System {}
impl Clock for Arc<System> {}

fn main() {}
