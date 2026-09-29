use crate::schema::{
    ParseLimits, SchemaAst, SchemaIssue, SchemaIssueKind, SchemaResource, SchemaShape,
};
use std::collections::BTreeSet;

#[derive(Clone, Copy)]
pub(crate) struct PreparedStructure {
    nodes: usize,
    depth: usize,
}

impl PreparedStructure {
    pub(crate) fn within(self, limits: ParseLimits) -> Result<(), SchemaIssue> {
        if self.nodes > limits.nodes.get() {
            return Err(SchemaIssue::new(SchemaIssueKind::LimitExceeded {
                resource: SchemaResource::Steps,
            }));
        }
        if self.depth > limits.depth.get() {
            return Err(SchemaIssue::new(SchemaIssueKind::LimitExceeded {
                resource: SchemaResource::Depth,
            }));
        }
        Ok(())
    }
}

enum Frame {
    Enter(SchemaAst, usize),
    Leave(usize),
}

/// Preparation counts every acyclic occurrence, including shared nodes.
/// A back-edge terminates at the recursive reference; value parsing still
/// applies its depth and node limits to each actual recursive value.
pub(crate) fn prepare_structure(
    root: &SchemaAst,
    limits: ParseLimits,
) -> Result<PreparedStructure, SchemaIssue> {
    let mut stack = vec![Frame::Enter(root.clone(), 1)];
    let mut active = BTreeSet::new();
    let mut nodes = 0usize;
    let mut depth = 0usize;
    while let Some(frame) = stack.pop() {
        match frame {
            Frame::Leave(identity) => {
                active.remove(&identity);
            }
            Frame::Enter(schema, current_depth) => {
                nodes = nodes
                    .checked_add(1)
                    .ok_or_else(|| limit(SchemaResource::Steps))?;
                if nodes > limits.nodes.get() {
                    return Err(limit(SchemaResource::Steps));
                }
                if current_depth > limits.depth.get() {
                    return Err(limit(SchemaResource::Depth));
                }
                depth = depth.max(current_depth);
                let identity = schema.identity();
                if !active.insert(identity) {
                    continue;
                }
                stack.push(Frame::Leave(identity));
                let has_children = if matches!(
                    schema.shape(),
                    SchemaShape::Opaque | SchemaShape::Primitive(_) | SchemaShape::Literal(_)
                ) {
                    false
                } else if let SchemaShape::Tuple(items) | SchemaShape::Union(items) = schema.shape()
                {
                    !items.is_empty()
                } else if let SchemaShape::Object { properties, .. } = schema.shape() {
                    !properties.is_empty()
                } else {
                    true
                };
                let child_depth = if has_children {
                    current_depth
                        .checked_add(1)
                        .ok_or_else(|| limit(SchemaResource::Depth))?
                } else {
                    current_depth
                };
                match schema.shape() {
                    SchemaShape::Opaque | SchemaShape::Primitive(_) | SchemaShape::Literal(_) => {}
                    SchemaShape::Sequence { element, .. } => {
                        stack.push(Frame::Enter(element.clone(), child_depth));
                    }
                    SchemaShape::Tuple(elements) | SchemaShape::Union(elements) => {
                        for element in elements.iter().rev() {
                            stack.push(Frame::Enter(element.clone(), child_depth));
                        }
                    }
                    SchemaShape::Object { properties, .. } => {
                        let mut names = BTreeSet::new();
                        for property in properties.iter().rev() {
                            if !names.insert(property.name()) {
                                return Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition));
                            }
                            stack.push(Frame::Enter(property.schema().clone(), child_depth));
                        }
                    }
                    SchemaShape::Reference(reference) => {
                        let target = reference
                            .resolve()
                            .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
                        stack.push(Frame::Enter(target, child_depth));
                    }
                    SchemaShape::Checked { source, name } => {
                        if name.is_empty() {
                            return Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition));
                        }
                        stack.push(Frame::Enter(source.clone(), child_depth));
                    }
                    SchemaShape::IntegerRange {
                        source,
                        minimum,
                        maximum,
                    } => {
                        if minimum > maximum {
                            return Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition));
                        }
                        stack.push(Frame::Enter(source.clone(), child_depth));
                    }
                    SchemaShape::UnsignedRange {
                        source,
                        minimum,
                        maximum,
                    } => {
                        if minimum > maximum {
                            return Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition));
                        }
                        stack.push(Frame::Enter(source.clone(), child_depth));
                    }
                    SchemaShape::StringLength {
                        source,
                        minimum,
                        maximum,
                        ..
                    } => {
                        if minimum > maximum {
                            return Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition));
                        }
                        stack.push(Frame::Enter(source.clone(), child_depth));
                    }
                    SchemaShape::StringPattern { source, .. }
                    | SchemaShape::EncodedView(source) => {
                        stack.push(Frame::Enter(source.clone(), child_depth));
                    }
                }
            }
        }
    }
    Ok(PreparedStructure { nodes, depth })
}

fn limit(resource: SchemaResource) -> SchemaIssue {
    SchemaIssue::new(SchemaIssueKind::LimitExceeded { resource })
}
