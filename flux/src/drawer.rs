use crate::{data, render, settings};
use render::{Buffer, Context, Framebuffer, Indices, Uniform, UniformValue, VertexBufferLayout};
use settings::Settings;

use web_sys::WebGl2RenderingContext as GL;
use web_sys::{WebGlBuffer, WebGlTransformFeedback, WebGlVertexArrayObject};
extern crate nalgebra_glm as glm;
use bytemuck::{Pod, Zeroable};
use std::rc::Rc;

static LINE_VERT_SHADER: &'static str = include_str!("./shaders/line.vert");
static LINE_FRAG_SHADER: &'static str = include_str!("./shaders/line.frag");
static ENDPOINT_VERT_SHADER: &'static str = include_str!("./shaders/endpoint.vert");
static ENDPOINT_FRAG_SHADER: &'static str = include_str!("./shaders/endpoint.frag");
static TEXTURE_VERT_SHADER: &'static str = include_str!("./shaders/texture.vert");
static TEXTURE_FRAG_SHADER: &'static str = include_str!("./shaders/texture.frag");
static PLACE_LINES_VERT_SHADER: &'static str = include_str!("./shaders/place_lines.vert");
static PLACE_LINES_FRAG_SHADER: &'static str = include_str!("./shaders/place_lines.frag");

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LineState {
    endpoint: [f32; 2],
    velocity: [f32; 2],
    color: [f32; 4],
    width: f32,
    opacity: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Projection {
    projection: [f32; 16],
    view: [f32; 16],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LineUniforms {
    line_width: f32,
    line_length: f32,
    line_begin_offset: f32,
    line_opacity: f32,
    line_fade_out_length: f32,
    timestep: f32,
    padding: [f32; 2],
    color_wheel: [f32; 24],
}

pub struct Drawer {
    context: Context,
    settings: Rc<Settings>,

    screen_width: u32,
    screen_height: u32,

    pub grid_width: u32,
    pub grid_height: u32,
    pub line_count: u32,

    line_state_buffer: Buffer,
    transform_feedback_buffer: WebGlTransformFeedback,
    // A dedicated buffer to write out the data from the transform feedback pass
    line_state_feedback_buffer: Buffer,

    place_lines_buffer: WebGlVertexArrayObject,
    draw_lines_buffer: WebGlVertexArrayObject,
    draw_endpoints_buffer: WebGlVertexArrayObject,
    draw_texture_buffer: WebGlVertexArrayObject,

    view_buffer: Buffer,
    line_uniforms: Buffer,

    place_lines_pass: render::Program,
    draw_lines_pass: render::Program,
    draw_endpoints_pass: render::Program,
    draw_texture_pass: render::Program,
    antialiasing_pass: render::MsaaPass,
}

impl Drawer {
    pub fn new(
        context: &Context,
        screen_width: u32,
        screen_height: u32,
        settings: &Rc<Settings>,
    ) -> Result<Self, render::Problem> {
        let (grid_width, grid_height) = compute_grid_size(screen_width, screen_height);

        let line_count =
            (grid_width / settings.grid_spacing) * (grid_height / settings.grid_spacing);
        // let line_state = data::new_line_state(grid_width, grid_height, settings.grid_spacing);
        let line_state = new_line_state(grid_width, grid_height, settings.grid_spacing);
        let line_state_buffer = Buffer::from_f32_array(
            &context,
            &bytemuck::cast_slice(&line_state),
            GL::ARRAY_BUFFER,
            GL::DYNAMIC_COPY,
        )?;
        let transform_feedback_buffer = context
            .create_transform_feedback()
            .ok_or(render::Problem::OutOfMemory)?;

        let line_vertices = Buffer::from_f32(
            &context,
            &data::LINE_VERTICES.to_vec(),
            GL::ARRAY_BUFFER,
            GL::STATIC_DRAW,
        )?;
        let basepoint_buffer = Buffer::from_f32(
            &context,
            &data::new_points(grid_width, grid_height, settings.grid_spacing),
            GL::ARRAY_BUFFER,
            GL::STATIC_DRAW,
        )?;
        let circle_vertices = Buffer::from_f32(
            &context,
            &data::new_semicircle(8),
            GL::ARRAY_BUFFER,
            GL::STATIC_DRAW,
        )?;
        let plane_vertices = Buffer::from_f32(
            &context,
            &data::PLANE_VERTICES.to_vec(),
            GL::ARRAY_BUFFER,
            GL::STATIC_DRAW,
        )?;
        let plane_indices = Buffer::from_u16(
            &context,
            &data::PLANE_INDICES.to_vec(),
            GL::ELEMENT_ARRAY_BUFFER,
            GL::STATIC_DRAW,
        )?;

        // Programs

        let place_lines_program = render::Program::new_with_transform_feedback(
            &context,
            (PLACE_LINES_VERT_SHADER, PLACE_LINES_FRAG_SHADER),
            &render::TransformFeedback {
                // The order here must match the order in the buffer!
                names: &[
                    "vEndpointVector",
                    "vVelocityVector",
                    "vColor",
                    "vLineWidth",
                    "vOpacity",
                ],
                mode: GL::INTERLEAVED_ATTRIBS,
            },
        )?;
        let draw_lines_program =
            render::Program::new(&context, (LINE_VERT_SHADER, LINE_FRAG_SHADER))?;
        let draw_endpoints_program =
            render::Program::new(&context, (ENDPOINT_VERT_SHADER, ENDPOINT_FRAG_SHADER))?;
        let draw_texture_program =
            render::Program::new(&context, (TEXTURE_VERT_SHADER, TEXTURE_FRAG_SHADER))?;

        // Pipelines

        let place_lines_buffer = render::create_vertex_array(
            &context,
            &place_lines_program,
            &[
                (
                    &basepoint_buffer,
                    VertexBufferLayout {
                        name: "basepoint",
                        size: 2,
                        type_: GL::FLOAT,
                        ..Default::default()
                    },
                ),
                (
                    &line_state_buffer,
                    VertexBufferLayout {
                        name: "iEndpointVector",
                        size: 2,
                        type_: GL::FLOAT,
                        stride: 10 * 4,
                        offset: 0 * 4,
                        divisor: 0,
                    },
                ),
                (
                    &line_state_buffer,
                    VertexBufferLayout {
                        name: "iVelocityVector",
                        size: 2,
                        type_: GL::FLOAT,
                        stride: 10 * 4,
                        offset: 2 * 4,
                        divisor: 0,
                    },
                ),
                (
                    &line_state_buffer,
                    VertexBufferLayout {
                        name: "iColor",
                        size: 4,
                        type_: GL::FLOAT,
                        stride: 10 * 4,
                        offset: 4 * 4,
                        divisor: 0,
                    },
                ),
                (
                    &line_state_buffer,
                    VertexBufferLayout {
                        name: "iLineWidth",
                        size: 1,
                        type_: GL::FLOAT,
                        stride: 10 * 4,
                        offset: 8 * 4,
                        divisor: 0,
                    },
                ),
                (
                    &line_state_buffer,
                    VertexBufferLayout {
                        name: "iOpacity",
                        size: 1,
                        type_: GL::FLOAT,
                        stride: 10 * 4,
                        offset: 9 * 4,
                        divisor: 0,
                    },
                ),
            ],
            None,
        )?;

        let draw_lines_buffer = render::create_vertex_array(
            &context,
            &draw_lines_program,
            &[
                (
                    &line_vertices,
                    VertexBufferLayout {
                        name: "lineVertex",
                        size: 2,
                        type_: GL::FLOAT,
                        ..Default::default()
                    },
                ),
                (
                    &basepoint_buffer,
                    VertexBufferLayout {
                        name: "basepoint",
                        size: 2,
                        type_: GL::FLOAT,
                        divisor: 1,
                        ..Default::default()
                    },
                ),
                (
                    &line_state_buffer,
                    VertexBufferLayout {
                        name: "iEndpointVector",
                        size: 2,
                        type_: GL::FLOAT,
                        stride: 10 * 4,
                        offset: 0 * 4,
                        divisor: 1,
                    },
                ),
                (
                    &line_state_buffer,
                    VertexBufferLayout {
                        name: "iVelocityVector",
                        size: 2,
                        type_: GL::FLOAT,
                        stride: 10 * 4,
                        offset: 2 * 4,
                        divisor: 1,
                    },
                ),
                (
                    &line_state_buffer,
                    VertexBufferLayout {
                        name: "iColor",
                        size: 4,
                        type_: GL::FLOAT,
                        stride: 10 * 4,
                        offset: 4 * 4,
                        divisor: 1,
                    },
                ),
                (
                    &line_state_buffer,
                    VertexBufferLayout {
                        name: "iLineWidth",
                        size: 1,
                        type_: GL::FLOAT,
                        stride: 10 * 4,
                        offset: 8 * 4,
                        divisor: 1,
                    },
                ),
                (
                    &line_state_buffer,
                    VertexBufferLayout {
                        name: "iOpacity",
                        size: 1,
                        type_: GL::FLOAT,
                        stride: 10 * 4,
                        offset: 9 * 4,
                        divisor: 1,
                    },
                ),
            ],
            None,
        )?;

        let draw_endpoints_buffer = render::create_vertex_array(
            &context,
            &draw_endpoints_program,
            &[
                (
                    &circle_vertices,
                    VertexBufferLayout {
                        name: "vertex",
                        size: 2,
                        type_: GL::FLOAT,
                        ..Default::default()
                    },
                ),
                (
                    &basepoint_buffer,
                    VertexBufferLayout {
                        name: "basepoint",
                        size: 2,
                        type_: GL::FLOAT,
                        divisor: 1,
                        ..Default::default()
                    },
                ),
                (
                    &line_state_buffer,
                    VertexBufferLayout {
                        name: "iEndpointVector",
                        size: 2,
                        type_: GL::FLOAT,
                        stride: 10 * 4,
                        offset: 0 * 4,
                        divisor: 1,
                    },
                ),
                (
                    &line_state_buffer,
                    VertexBufferLayout {
                        name: "iVelocityVector",
                        size: 2,
                        type_: GL::FLOAT,
                        stride: 10 * 4,
                        offset: 2 * 4,
                        divisor: 1,
                    },
                ),
                (
                    &line_state_buffer,
                    VertexBufferLayout {
                        name: "iColor",
                        size: 4,
                        type_: GL::FLOAT,
                        stride: 10 * 4,
                        offset: 4 * 4,
                        divisor: 1,
                    },
                ),
                (
                    &line_state_buffer,
                    VertexBufferLayout {
                        name: "iLineWidth",
                        size: 1,
                        type_: GL::FLOAT,
                        stride: 10 * 4,
                        offset: 8 * 4,
                        divisor: 1,
                    },
                ),
                (
                    &line_state_buffer,
                    VertexBufferLayout {
                        name: "iOpacity",
                        size: 1,
                        type_: GL::FLOAT,
                        stride: 10 * 4,
                        offset: 9 * 4,
                        divisor: 1,
                    },
                ),
            ],
            None,
        )?;

        // Uniforms

        let projection_matrix = new_projection_matrix(grid_width, grid_height);

        let view_matrix = glm::scale(
            &glm::identity(),
            &glm::vec3(settings.view_scale, settings.view_scale, 1.0),
        );

        let projection = Projection {
            projection: projection_matrix.as_slice().try_into().unwrap(),
            view: view_matrix.as_slice().try_into().unwrap(),
        };
        let view_buffer = Buffer::from_f32_array(
            &context,
            &bytemuck::cast_slice(&[projection]),
            GL::ARRAY_BUFFER,
            GL::STATIC_DRAW,
        )?;

        let uniforms = LineUniforms {
            line_width: settings.line_width,
            line_length: settings.line_length,
            line_begin_offset: settings.line_begin_offset,
            line_opacity: settings.line_opacity,
            line_fade_out_length: settings.line_fade_out_length,
            timestep: 0.0,
            padding: [0.0, 0.0],
            color_wheel: settings::color_wheel_from_scheme(&settings.color_scheme),
        };
        let line_uniforms = Buffer::from_f32_array(
            &context,
            &bytemuck::cast_slice(&[uniforms]),
            GL::ARRAY_BUFFER,
            GL::STATIC_DRAW,
        )?;

        place_lines_program.set_uniform_block("Projection", 0);
        place_lines_program.set_uniform_block("LineUniforms", 1);
        draw_lines_program.set_uniform_block("Projection", 0);
        draw_lines_program.set_uniform_block("LineUniforms", 1);
        draw_endpoints_program.set_uniform_block("Projection", 0);
        draw_endpoints_program.set_uniform_block("LineUniforms", 1);

        let draw_texture_buffer = render::create_vertex_array(
            &context,
            &draw_texture_program,
            &[(
                &plane_vertices,
                VertexBufferLayout {
                    name: "position",
                    size: 3,
                    type_: GL::FLOAT,
                    ..Default::default()
                },
            )],
            Some(&plane_indices),
        )?;

        let antialiasing_samples = 4;
        let antialiasing_pass =
            render::MsaaPass::new(context, screen_width, screen_height, antialiasing_samples)?;

        Ok(Self {
            context: Rc::clone(context),
            settings: Rc::clone(settings),

            screen_width,
            screen_height,
            grid_width,
            grid_height,
            line_count,

            line_state_buffer,
            line_state_feedback_buffer: Buffer::from_f32_array(
                &context,
                &bytemuck::cast_slice(&line_state),
                GL::ARRAY_BUFFER,
                GL::DYNAMIC_READ,
            )?,
            transform_feedback_buffer,

            place_lines_buffer,
            draw_lines_buffer,
            draw_endpoints_buffer,
            draw_texture_buffer,

            view_buffer,
            line_uniforms,

            place_lines_pass: place_lines_program,
            draw_lines_pass: draw_lines_program,
            draw_endpoints_pass: draw_endpoints_program,
            draw_texture_pass: draw_texture_program,
            antialiasing_pass,
        })
    }

    pub fn update_settings(&mut self, new_settings: &Rc<Settings>) -> () {
        // Rename to update
        // self.settings = new_settings.clone();
        // self.color_wheel = settings::color_wheel_from_scheme(&new_settings.color_scheme);
    }

    pub fn resize(&mut self, width: u32, height: u32) -> () {
        let (grid_width, grid_height) = compute_grid_size(width, height);

        self.screen_width = width;
        self.screen_height = height;
        self.grid_width = grid_width;
        self.grid_height = grid_height;

        // self.projection_matrix = new_projection_matrix(grid_width, grid_height);
        self.antialiasing_pass.resize(width, height);
    }

    pub fn place_lines(&self, timestep: f32, texture: &Framebuffer) -> () {
        self.context
            .viewport(0, 0, self.screen_width as i32, self.screen_height as i32);
        self.context.disable(GL::BLEND);

        self.place_lines_pass.use_program();
        self.context
            .bind_vertex_array(Some(&self.place_lines_buffer));

        self.context
            .bind_buffer(GL::UNIFORM_BUFFER, Some(&self.line_uniforms.id));
        self.context
            .buffer_sub_data_with_i32_and_u8_array_and_src_offset_and_length(
                GL::UNIFORM_BUFFER,
                5 * 4,
                &timestep.to_ne_bytes(),
                0,
                4,
            );
        self.context.bind_buffer(GL::UNIFORM_BUFFER, None);

        self.context
            .bind_buffer_base(GL::UNIFORM_BUFFER, 0, Some(&self.view_buffer.id));
        self.context
            .bind_buffer_base(GL::UNIFORM_BUFFER, 1, Some(&self.line_uniforms.id));

        self.place_lines_pass.set_uniform(&Uniform {
            name: "velocityTexture",
            value: UniformValue::Texture2D(&texture.texture, 0),
        });

        self.context.bind_transform_feedback(
            GL::TRANSFORM_FEEDBACK,
            Some(&self.transform_feedback_buffer),
        );
        self.context.bind_buffer_base(
            GL::TRANSFORM_FEEDBACK_BUFFER,
            0,
            Some(&self.line_state_feedback_buffer.id),
        );

        self.context.enable(GL::RASTERIZER_DISCARD);
        self.context.begin_transform_feedback(GL::POINTS);

        self.context
            .draw_arrays(GL::POINTS, 0, self.line_count as i32);

        self.context.end_transform_feedback();
        self.context
            .bind_buffer_base(GL::TRANSFORM_FEEDBACK_BUFFER, 0, None);
        self.context
            .bind_buffer(GL::COPY_WRITE_BUFFER, Some(&self.line_state_buffer.id));
        self.context.bind_buffer(
            GL::COPY_READ_BUFFER,
            Some(&self.line_state_feedback_buffer.id),
        );
        // Copy new line state
        self.context.copy_buffer_sub_data_with_i32_and_i32_and_i32(
            GL::COPY_READ_BUFFER,
            GL::COPY_WRITE_BUFFER,
            0,
            0,
            (std::mem::size_of::<LineState>() as i32) * (self.line_count as i32),
        );
        self.context.bind_buffer(GL::COPY_READ_BUFFER, None);
        self.context.bind_buffer(GL::COPY_WRITE_BUFFER, None);
        self.context
            .bind_transform_feedback(GL::TRANSFORM_FEEDBACK, None);
        self.context.disable(GL::RASTERIZER_DISCARD);
    }

    pub fn draw_lines(&self) -> () {
        self.context
            .viewport(0, 0, self.screen_width as i32, self.screen_height as i32);

        self.context.enable(GL::BLEND);
        self.context.blend_func(GL::SRC_ALPHA, GL::ONE);

        self.draw_lines_pass.use_program();
        self.context
            .bind_vertex_array(Some(&self.draw_lines_buffer));

        self.context
            .bind_buffer_base(GL::UNIFORM_BUFFER, 0, Some(&self.view_buffer.id));
        self.context
            .bind_buffer_base(GL::UNIFORM_BUFFER, 1, Some(&self.line_uniforms.id));

        self.context
            .draw_arrays_instanced(GL::TRIANGLES, 0, 6, self.line_count as i32);

        self.context.disable(GL::BLEND);
    }

    pub fn draw_endpoints(&self) -> () {
        self.context
            .viewport(0, 0, self.screen_width as i32, self.screen_height as i32);

        self.context.enable(GL::BLEND);
        self.context.blend_func(GL::SRC_ALPHA, GL::ONE);

        self.draw_endpoints_pass.use_program();
        self.context
            .bind_vertex_array(Some(&self.draw_endpoints_buffer));

        self.context
            .bind_buffer_base(GL::UNIFORM_BUFFER, 0, Some(&self.view_buffer.id));
        self.context
            .bind_buffer_base(GL::UNIFORM_BUFFER, 1, Some(&self.line_uniforms.id));

        self.context
            .draw_arrays_instanced(GL::TRIANGLE_FAN, 0, 10, self.line_count as i32);

        self.context.disable(GL::BLEND);
    }

    #[allow(dead_code)]
    pub fn draw_texture(&self, texture: &Framebuffer) -> () {
        self.context
            .viewport(0, 0, self.screen_width as i32, self.screen_height as i32);

        self.draw_texture_pass.use_program();

        self.context
            .bind_vertex_array(Some(&self.draw_texture_buffer));

        self.context.active_texture(GL::TEXTURE0);
        self.context
            .bind_texture(GL::TEXTURE_2D, Some(&texture.texture));

        self.context
            .draw_elements_with_i32(GL::TRIANGLES, 6, GL::UNSIGNED_SHORT, 0);
    }

    pub fn with_antialiasing<T>(&self, draw_call: T) -> ()
    where
        T: Fn() -> (),
    {
        self.antialiasing_pass.draw_to(draw_call);
    }
}

fn compute_grid_size(width: u32, height: u32) -> (u32, u32) {
    let base_units = 1000;
    let aspect_ratio: f32 = (width as f32) / (height as f32);

    // landscape
    if aspect_ratio > 1.0 {
        (base_units, ((base_units as f32) / aspect_ratio) as u32)

    // portrait
    } else {
        (((base_units as f32) * aspect_ratio) as u32, base_units)
    }
}

fn new_projection_matrix(width: u32, height: u32) -> glm::TMat4<f32> {
    let half_width = (width as f32) / 2.0;
    let half_height = (height as f32) / 2.0;

    glm::ortho(
        -half_width,
        half_width,
        -half_height,
        half_height,
        -1.0,
        1.0,
    )
}

// World space coordinates: zero-centered, width x height
fn new_line_state(width: u32, height: u32, grid_spacing: u32) -> Vec<LineState> {
    let rows = height / grid_spacing;
    let cols = width / grid_spacing;
    let mut data =
        Vec::with_capacity(std::mem::size_of::<LineState>() / 4 * (rows * cols) as usize);

    for _ in 0..rows {
        for _ in 0..cols {
            data.push(LineState {
                endpoint: [0.001, 0.001], // investigate
                velocity: [0.01, 0.01],
                color: [0.0, 0.0, 0.0, 0.0],
                width: 0.0,
                opacity: 0.0,
            });
        }
    }

    data
}
