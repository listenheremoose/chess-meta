use std::collections::HashMap;

use iced::mouse;
use iced::widget::canvas::{self, Canvas, Geometry, Path, Stroke, Text};
use iced::{Color, Element, Length, Point, Rectangle, Renderer, Size, Theme};

use crate::app::Message;
use crate::coordinator::TreeSnapshot;
use crate::search::{NodeId, NodeType};
use crate::ui::colors;

/// State for the tree view canvas.
#[derive(Default)]
pub struct TreeViewState {
    cache: canvas::Cache,
}

impl TreeViewState {
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }
}

/// Right panel: search tree visualization.
pub fn view<'a>(
    snapshot: Option<&TreeSnapshot>,
    state: &'a TreeViewState,
    min_visits: u64,
    max_depth: u32,
) -> Element<'a, Message> {
    let snapshot_clone = snapshot.cloned();
    Canvas::new(TreeViewProgram {
        snapshot: snapshot_clone,
        min_visits,
        max_depth,
        cache: &state.cache,
    })
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

struct TreeViewProgram<'a> {
    snapshot: Option<TreeSnapshot>,
    min_visits: u64,
    max_depth: u32,
    cache: &'a canvas::Cache,
}

impl<'a> canvas::Program<Message> for TreeViewProgram<'a> {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let geometry = self.cache.draw(renderer, bounds.size(), |frame| {
            // Dark background
            frame.fill_rectangle(
                Point::ORIGIN,
                bounds.size(),
                colors::BACKGROUND,
            );

            let snapshot = match &self.snapshot {
                Some(s) => s,
                None => {
                    let text = Text {
                        content: "No search data".to_string(),
                        position: Point::new(bounds.width / 2.0, bounds.height / 2.0),
                        color: colors::TEXT_DIM,
                        size: iced::Pixels(14.0),
                        ..Text::default()
                    };
                    frame.fill_text(text);
                    return;
                }
            };

            if snapshot.nodes.is_empty() {
                return;
            }

            // Filter nodes by min visits and max depth
            let visible: Vec<_> = snapshot
                .nodes
                .iter()
                .filter(|node| node.visit_count >= self.min_visits && node.depth <= self.max_depth)
                .collect();

            if visible.is_empty() {
                return;
            }

            // Layout: simple layered tree
            // Group by depth
            let max_depth = match visible.iter().map(|node| node.depth).max() {
                Some(depth) => depth,
                None => 0,
            };
            let level_height = bounds.height / (max_depth as f32 + 2.0);

            // Assign x positions per level
            let mut level_counts: HashMap<u32, usize> = HashMap::new();
            let mut positions: HashMap<NodeId, Point> = HashMap::new();

            // Count nodes per level first
            visible.iter().for_each(|node| {
                *level_counts.entry(node.depth).or_insert(0) += 1;
            });

            let mut level_indices: HashMap<u32, usize> = HashMap::new();

            visible.iter().for_each(|node| {
                let count = level_counts[&node.depth];
                let index = level_indices.entry(node.depth).or_insert(0);
                let x = bounds.width * (*index as f32 + 1.0) / (count as f32 + 1.0);
                let y = level_height * (node.depth as f32 + 1.0);
                positions.insert(node.id, Point::new(x, y));
                *index += 1;
            });

            // Draw edges
            visible.iter()
                .filter_map(|node| node.parent_id.map(|parent_id| (node, parent_id)))
                .for_each(|(node, parent_id)| {
                    match (positions.get(&node.id), positions.get(&parent_id)) {
                        (Some(&child_pos), Some(&parent_pos)) => {
                            let edge = Path::line(parent_pos, child_pos);
                            frame.stroke(
                                &edge,
                                Stroke::default()
                                    .with_color(Color {
                                        a: 0.3,
                                        ..colors::TEXT_DIM
                                    })
                                    .with_width(1.0),
                            );
                        }
                        _ => {}
                    }
                });

            // Draw nodes
            let max_visits = match visible.iter().map(|node| node.visit_count).max() {
                Some(max_visit_count) => max_visit_count,
                None => 1,
            } as f32;

            visible.iter()
                .filter_map(|node| positions.get(&node.id).map(|&pos| (node, pos)))
                .for_each(|(node, pos)| {
                    let size = 4.0 + 16.0 * (node.visit_count as f32 / max_visits).sqrt();

                    // Color by Q value: green (good for us) to red (bad)
                    let q_value = node.q_value as f32;
                    let node_color = match node.node_type {
                        NodeType::Max => lerp_color(colors::RED, colors::GREEN, q_value),
                        NodeType::Chance => lerp_color(colors::RED, colors::ORANGE, q_value),
                    };

                    match node.node_type {
                        NodeType::Max => draw_max_node(frame, pos, size, node_color),
                        NodeType::Chance => draw_chance_node(frame, pos, size, node_color),
                    }

                    // Label for nodes with enough visits
                    if node.visit_count as f32 > max_visits * 0.05 {
                        match &node.move_uci {
                            Some(uci_move) => {
                                let label = Text {
                                    content: uci_move.clone(),
                                    position: Point::new(pos.x, pos.y + size / 2.0 + 2.0),
                                    color: colors::TEXT,
                                    size: iced::Pixels(10.0),
                                    ..Text::default()
                                };
                                frame.fill_text(label);
                            }
                            None => {}
                        }
                    }
                });
        });

        vec![geometry]
    }
}

fn draw_max_node(frame: &mut canvas::Frame, pos: Point, size: f32, color: Color) {
    frame.fill_rectangle(
        Point::new(pos.x - size / 2.0, pos.y - size / 2.0),
        Size::new(size, size),
        color,
    );
}

fn draw_chance_node(frame: &mut canvas::Frame, pos: Point, size: f32, color: Color) {
    let circle = Path::circle(pos, size / 2.0);
    frame.fill(&circle, color);
}

fn lerp_color(from_color: Color, to_color: Color, lerp_factor: f32) -> Color {
    let lerp_factor = lerp_factor.clamp(0.0, 1.0);
    Color::from_rgba(
        from_color.r + (to_color.r - from_color.r) * lerp_factor,
        from_color.g + (to_color.g - from_color.g) * lerp_factor,
        from_color.b + (to_color.b - from_color.b) * lerp_factor,
        0.9,
    )
}
