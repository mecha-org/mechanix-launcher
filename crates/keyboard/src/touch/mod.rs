//! Touch handling for the on-screen keyboard, kept free of widgets: resolving
//! a touch point to a key lives here.

mod resolve;
mod tracker;

#[cfg(test)]
mod sim;
