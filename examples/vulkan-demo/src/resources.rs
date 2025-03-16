use std::{
    fs::read,
    io::{BufReader, Cursor},
    path::{Path, PathBuf},
};

use tobj::{Material, Model};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to access model file: {0}")]
    Io(#[from] std::io::Error),
    #[error("Failed to load model data: {0}")]
    ObjLoad(#[from] tobj::LoadError),
}

pub struct ModelData {
    pub dir: PathBuf,
    pub models: Vec<Model>,
    pub materials: Vec<Material>,
}

impl ModelData {
    fn file_io_error(msg: &str) -> Error {
        Error::Io(std::io::Error::new(std::io::ErrorKind::NotFound, msg))
    }

    pub fn from_path(model_file_path: &Path) -> Result<Self> {
        let base_dir = model_file_path
            .parent()
            .ok_or(Self::file_io_error("Failed to get parent directory"))?;
        let model_text = read(model_file_path).expect("Failed to read model file");

        let obj_cursor = Cursor::new(model_text);
        let mut obj_reader = BufReader::new(obj_cursor);
        let (models, obj_materials) = tobj::load_obj_buf(
            &mut obj_reader,
            &tobj::LoadOptions {
                triangulate: true,
                single_index: true,
                ..Default::default()
            },
            |p| {
                let p = base_dir.join(p);
                let mat_text =
                    read(&p).unwrap_or_else(|_| panic!("Failed to read material file: {:?}", p));
                tobj::load_mtl_buf(&mut BufReader::new(Cursor::new(mat_text)))
            },
        )?;

        let obj_materials = obj_materials?;
        Ok(Self {
            dir: base_dir.to_path_buf(),
            models,
            materials: obj_materials,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::Path;

    #[test]
    fn test_model() {
        let model_path = Path::new("assets/models/cube/cube.obj");
        let model_data = ModelData::from_path(model_path).unwrap();

        assert_eq!(model_data.models.len(), 1);
        assert_eq!(model_data.materials.len(), 1);

        let mesh = &model_data.models[0];
        assert_eq!(mesh.mesh.positions.len(), 831);
        assert_eq!(mesh.mesh.texcoords.len(), 554);
        assert_eq!(mesh.mesh.indices.len(), 1284);
        assert_eq!(mesh.mesh.material_id, Some(0));

        let material = &model_data.materials[0];
        assert_eq!(
            material.diffuse_texture,
            Some("cube-diffuse.jpg".to_string())
        );
        assert_eq!(material.normal_texture, Some("cube-normal.png".to_string()));
    }
}
