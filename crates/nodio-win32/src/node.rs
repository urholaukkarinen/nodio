use nodio_core::Uuid;

#[derive(Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
pub enum NodeConnectionKind {
    DefaultEndpoint,
    Loopback,
    Listen,
}

#[derive(Debug, Copy, Clone)]
pub struct NodeConnectionInfo {
    pub id: Uuid,
    pub src_id: Uuid,
    pub dst_id: Uuid,
    pub kind: NodeConnectionKind,
}
