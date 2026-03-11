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
                .filter(|n| n.visit_count >= self.min_visits && n.depth <= self.max_depth)
                .collect();

            if visible.is_empty() {
                return;
            }

            // Layout: simple layered tree
            // Group by depth
            let max_depth = match visible.iter().map(|n| n.depth).max() {
                Some(d) => d,
                None => 0,
            };
            let level_height = bounds.height / (max_depth as f32 + 2.0);

            // Assign x positions per level
            let mut level_counts: HashMap<u32, usize> = HashMap::new();
            let mut positions: HashMap<NodeId, Point> = HashMap::new();

            // Count nodes per level first
            visible.iter().for_each(|n| {
                *level_counts.entry(n.depth).or_insert(0) += 1;
            });

            let mut level_indices: HashMap<u32, usize> = HashMap::new();

            visible.iter().for_each(|n| {
                let count = level_counts[&n.depth];
                let idx = level_indices.entry(n.depth).or_insert(0);
                let x = bounds.width * (*idx as f32 + 1.0) / (count as f32 + 1.0);
                let y = level_height * (n.depth as f32 + 1.0);
                positions.insert(n.id, Point::new(x, y));
                *idx += 1;
            });

            // Draw edges
            visible.iter()
                .filter_map(|n| n.parent_id.map(|pid| (n, pid)))
                .for_each(|(n, parent_id)| {
                    match (positions.get(&n.id), positions.get(&parent_id)) {
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
            let max_visits = match visible.iter().map(|n| n.visit_count).max() {
                Some(v) => v,
                None => 1,
            } as f32;

            visible.iter()
                .filter_map(|n| positions.get(&n.id).map(|&pos| (n, pos)))
                .for_each(|(n, pos)| {
                    let size = 4.0 + 16.0 * (n.visit_count as f32 / max_visits).sqrt();

                    // Color by Q value: green (good for us) to red (bad)
                    let q = n.q_value as f32;
                    let node_color = match n.node_type {
                        NodeType::Max => lerp_color(colors::RED, colors::GREEN, q),
                        NodeType::Chance => lerp_color(colors::RED, colors::ORANGE, q),
                    };

                    match n.node_type {
                        NodeType::Max => draw_max_node(frame, pos, size, node_color),
                        NodeType::Chance => draw_chance_node(frame, pos, size, node_color),
                    }

                    // Label for nodes with enough visits
                    if n.visit_count as f32 > max_visits * 0.05 {
                        match &n.move_uci {
                            Some(uci) => {
                                let label = Text {
                                    content: uci.clone(),
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

fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    Color::from_rgba(
        a.r + (b.r - a.r) * t,
        a.g + (b.g - a.g) * t,
        a.b + (b.b - a.b) * t,
        0.9,
    )
}
