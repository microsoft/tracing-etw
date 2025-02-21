mod subscriber;

use std::{pin::Pin, sync::Arc};

use crate::native::OutputMode;

pub struct _EtwTracingSubscriber<OutMode: OutputMode> {
    pub(crate) provider: Pin<Arc<crate::native::Provider<OutMode>>>,
    pub(crate) default_keyword: u64,
}
