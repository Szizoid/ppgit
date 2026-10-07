//! Integration tests: every test runs the real `pp` binary in a throwaway
//! directory, with local bare repositories standing in for the GitHub
//! remotes and a mock `gh` on PATH — no test touches the network. One
//! test binary rather than one per file, so the harness compiles once.

mod harness;

mod branch;
mod cherry_pick;
mod clone;
mod commit;
mod doctor;
mod doctor_fix;
mod dual;
mod hooks;
mod init;
mod mv;
mod notes;
mod private_message;
mod privatize;
mod pull;
mod push;
mod rebase;
mod reset;
mod tree;
