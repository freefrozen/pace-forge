//! 业务接口层。

pub mod accounts;
pub mod audit;
pub mod egress;
pub mod start_time;
pub mod client;
pub mod cheat;
pub mod coord;
pub mod elev;
pub mod device_table;
pub mod crosscheck;
pub mod fence;
pub mod obs;
pub mod points;
pub mod policy;
pub mod flow;
pub mod gmttime;
pub mod guard;
pub mod guardrails;
pub mod gt4;
pub mod gt4image;
pub mod login;
pub mod records;
pub mod risk;
pub mod semester;
pub mod session;
pub mod submit;

pub use client::{Client, Credentials, DeviceProfile};
pub use login::{LoginOutcome, UserProfile};
