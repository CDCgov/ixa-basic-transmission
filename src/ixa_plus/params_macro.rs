#[macro_export]
macro_rules! define_parameters {
    (
        defaults: $default_file:expr,
        $(#[$meta:meta])*
        $vis:vis struct $name:ident {
            $(
                $(#[$field_meta:meta])*
                $field_name:ident : $field_type:ty $({
                    $( default : $default_value:expr, )?
                    $( validate($validate_arg:ident) $validate_body:block )?
                })?
            ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
        pub struct $name {
            $(
                $(#[$field_meta])*
                pub $field_name: $field_type,
            )*
        }


        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                let json = toml::to_string(&self).unwrap();
                write!(f, "{}", json)
            }
        }

        paste::paste! {
            impl $name {
                pub fn builder() -> [<$name Builder>] {
                    [<$name Builder>]::default()
                }
            }
            static DEFAULT_PARAMS: LazyLock<[<$name Builder>]> = LazyLock::new(|| {
                toml::from_str::<[<$name Builder>]>(include_str!($default_file))
                    .expect("Failed to parse default parameters")
            });

            impl Default for $name {
                fn default() -> Self {
                    [<$name Builder>]::default().try_into().unwrap()
                }
            }

            #[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
            pub struct [<$name Builder>] {
                $(
                    $(#[$field_meta])*
                    $field_name: Option<$field_type>,
                )*
            }


            impl $crate::ixa_plus::params_macro::IxaParameters for $name {
                type Builder = [<$name Builder>];
            }

            impl $crate::ixa_plus::params_macro::IxaParametersBuilder<$name> for [<$name Builder>] {
                fn extend_from(self, other: Self) -> Self {
                    Self {
                        $(
                            $field_name: self.$field_name.or(other.$field_name),
                        )*
                    }
                }
                fn build(self) -> Result<$name, anyhow::Error> {
                    self.build()
                }
            }

            impl [<$name Builder>] {
                $(
                    #[inline]
                    #[allow(dead_code)]
                    pub fn $field_name(mut self, value: $field_type) -> Self {
                        self.$field_name = Some(value);
                        self
                    }
                )*


                $(

                    #[inline]
                    fn [<validate_ $field_name>](_value: &$field_type) -> Result<(), anyhow::Error> {
                        $($(
                        let $validate_arg = _value;
                        $validate_body
                        )?)?

                        Ok(())
                    }

                )*


                $(
                    #[inline]
                    #[allow(unreachable_code)]
                    fn [<build_ $field_name>](value: Option<$field_type>) -> Result<$field_type, anyhow::Error> {
                        if let Some(value) = value {
                            Self::[<validate_ $field_name>](&value)
                                .map_err(|e| anyhow::anyhow!(
                                    concat!("Validation failed for parameter ", stringify!($field_name), ": ",)
                                        .to_string() + &e.to_string()
                                ))?;

                            return Ok(value);
                        }

                        $($(
                            return Ok($default_value);
                        )?)?

                        Err(anyhow::anyhow!(concat!("Missing value for parameter ", stringify!($field_name)).to_string()))
                    }
                )*
                fn build(self) -> Result<$name, anyhow::Error> {
                    Ok($name {
                        $(
                            $field_name: Self::[<build_ $field_name>](self.$field_name).map_err(|e| $crate::IxaError::IxaError(e.to_string()))?,
                        )*
                    })
                }
            }
            impl Default for [<$name Builder>] {
                fn default() -> Self {
                    DEFAULT_PARAMS.clone()
                }
            }

            impl TryFrom<[<$name Builder>]> for $name {
                type Error = anyhow::Error;
                fn try_from(builder: [<$name Builder>]) -> Result<Self, anyhow::Error> {
                    builder.build()
                }
            }

            ixa::define_global_property!(GlobalParams, $name);

            pub trait ParametersExt: ixa::PluginContext {
                fn use_default_params(&mut self) -> &Params {
                    self.set_global_property_value(GlobalParams, $name::default()).expect("Failed to set GlobalParams to default");
                    self.params()
                }
                fn set_params(&mut self, params: $name) -> &Params {
                    self.set_global_property_value(GlobalParams, params).expect("Failed to set GlobalParams");
                    self.params()
                }
                fn params(&self) -> &Params {
                    self.get_global_property_value(GlobalParams)
                        .expect("Expected GlobalParams to be set")
                }
                $(
                    #[inline]
                    #[allow(dead_code)]
                    fn [<param_ $field_name>](&self) -> &$field_type {
                        &self.params().$field_name
                    }
                )*
            }
            impl<C> ParametersExt for C where C: PluginContext {}
        }
    };
}

pub trait IxaParametersBuilder<P: IxaParameters>:
    Sized + serde::de::DeserializeOwned + Default
{
    fn extend_from(self, other: Self) -> Self;
    fn build(self) -> Result<P, anyhow::Error>;
}

pub trait IxaParameters: Sized + serde::Serialize + serde::de::DeserializeOwned {
    type Builder: IxaParametersBuilder<Self>;
    fn builder() -> Self::Builder {
        Self::Builder::default()
    }
    fn from_args() -> Option<Self> {
        let args = std::env::args().collect::<Vec<_>>();
        // File --params <path>
        let mut prev_arg: Option<&str> = None;
        for arg in &args[1..] {
            if let Some(prev) = prev_arg {
                if prev == "--params" {
                    return Some(
                        Self::try_from_file(arg).expect("Could not parse parameters from file"),
                    );
                }
            }
            prev_arg = Some(arg);
        }
        None
    }
    // Parse parameters from toml or json
    fn try_from_file<P: AsRef<std::path::Path>>(path: P) -> anyhow::Result<Self> {
        let contents = std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("{}: {}", path.as_ref().display(), e))?;

        let file_params: Self::Builder =
            if path.as_ref().extension().and_then(|s| s.to_str()) == Some("json") {
                serde_json::from_str(&contents)?
            } else if path.as_ref().extension().and_then(|s| s.to_str()) == Some("toml") {
                log::info!("Loading parameters from file {}", path.as_ref().display());
                toml::from_str(&contents)?
            } else {
                anyhow::bail!("Unsupported config file format. Use .toml or .json");
            };

        // File params should extend default params
        let params = file_params.extend_from(Self::Builder::default());

        Ok(params.build()?)
    }
}
