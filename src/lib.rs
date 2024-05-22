use std::path::PathBuf;

use crate::types::output::Localizable;

pub fn parse_from_string(raw: String) -> Localizable {
    parse::from_string(raw)
}

pub fn parse_from_bytes(raw: &[u8]) -> Localizable {
    parse_from_string(String::from_utf8(raw.to_vec()).unwrap())
}

pub fn parse_from_dir(dir: &PathBuf) -> Localizable {
    parse_from_string(std::fs::read_to_string(dir).unwrap())
}

mod parse {
    use std::collections::HashMap;

    use crate::types::input::{StringUnit, Translation};
    use crate::types::output::{Localizable, LocalizationValue, PluralVariate, SinglePluralVariation, SingleTranslation, TranslationValue};

    pub(crate) fn from_string(translations: String) -> Localizable {
        let translations: Translation = serde_json::from_str(&translations).unwrap();
        let mut localizable = Localizable::default();

        for (key, language) in &translations.strings {
            let mut localization_value = LocalizationValue::default();

            for (language, map) in &language.localizations {
                assert_eq!(map.len(), 1);

                let key = map.keys().collect::<Vec<_>>().remove(0).to_string();
                let value = map.get(&key).unwrap().clone();

                let translation = if key == "stringUnit" {
                    let translation_value = extract_translation(&value);

                    crate::types::output::Translation::Localization(translation_value)
                } else {
                    assert_eq!(key, "variations");

                    let variations: HashMap<String, serde_json::Value> = serde_json::from_value(value).unwrap();

                    assert_eq!(1, variations.len());

                    let value = variations.get("plural").unwrap().clone();
                    let plural: HashMap<String, serde_json::Value> = serde_json::from_value(value).unwrap();
                    let mut variations: Vec<_> = Default::default();

                    for (variation, object) in plural {
                        let variate = match variation.as_str() {
                            "one" => PluralVariate::One,
                            "other" => PluralVariate::Other,
                            _ => panic!("Unknown variant: {:#?}", variation)
                        };

                        let translation_value = extract_translation(object.as_object().unwrap().get("stringUnit").unwrap());

                        variations.push(SinglePluralVariation {
                            variate,
                            translation_value,
                        });
                    }

                    variations.sort_by(|a, b| a.variate.android_key().cmp(&b.variate.android_key()));

                    crate::types::output::Translation::PluralVariation(variations)
                };

                localization_value.language_translation.insert(language.to_string(), translation);
            }

            if localization_value.language_translation.get("en").is_none() {
                panic!("The key is the translation, this is not supported because it makes parsing difficult. Key: {key}");
            }

            assert!(!key.contains(" "), "Keys should not contain whitespaces: {key}");

            localizable.single_translation.push(SingleTranslation {
                key: key.to_string(),
                localization_value,
            });
        }

        localizable
    }

    fn extract_translation(string_unit: &serde_json::Value) -> TranslationValue {
        let string_unit: StringUnit = serde_json::from_value(string_unit.clone()).unwrap_or_else(|err| panic!("Got err: {:#?} for struct: {:#?}", err, string_unit));

        TranslationValue {
            raw: string_unit.value,
        }
    }
}

mod types {
    pub(crate) mod input {
        use std::collections::HashMap;

        use serde::Deserialize;

        #[derive(Debug, Deserialize, Clone)]
        pub(crate) struct Translation {
            pub(crate) strings: HashMap<String, Language>,
        }

        #[derive(Debug, Deserialize, Clone)]
        pub(crate) struct Language {
            // I made this Value to use it later on, I can not use StringUnit because there are variations which needs to be handled
            pub(crate) localizations: HashMap<String, HashMap<String, serde_json::Value>>,
        }

        #[derive(Debug, Deserialize, Clone)]
        pub(crate) struct StringUnit {
            pub(crate) value: String,
            pub(crate) state: String,
        }
    }

    pub mod output {
        use std::collections::HashMap;

        #[derive(Debug, Clone, Default)]
        pub struct Localizable {
            pub single_translation: Vec<SingleTranslation>,
        }

        #[derive(Debug, Clone)]
        pub struct SingleTranslation {
            pub key: String,
            pub localization_value: LocalizationValue,
        }

        #[derive(Debug, Clone)]
        pub struct SingleLocalizedPerLanguage {
            pub key: String,
            pub translation: Translation,
        }

        impl SingleLocalizedPerLanguage {
            pub fn sanitize_key_for_android(&self) -> String {
                self.key.replace("-", "_")
            }
        }

        #[derive(Debug, Clone, Default)]
        pub struct LocalizedPerLanguage {
            pub language_localized: HashMap<String, Vec<SingleLocalizedPerLanguage>>,
        }

        impl Localizable {
            pub fn localized_per_language(&self) -> LocalizedPerLanguage {
                let mut localized_per_language = LocalizedPerLanguage::default();

                for single_translation in &self.single_translation {
                    for (language, translation) in &single_translation.localization_value.language_translation {
                        let mut single_localized_per_language = localized_per_language.language_localized.entry(language.to_string()).or_default();

                        single_localized_per_language.push(SingleLocalizedPerLanguage {
                            key: single_translation.key.to_string(),
                            translation: translation.clone(),
                        });
                    }
                }

                localized_per_language
            }
        }

        #[derive(Debug, Clone, Default)]
        pub struct AndroidLocalizeConfig {
            pub shared_app_name: String,
        }

        impl LocalizedPerLanguage {
            pub fn localized_for_android(&self, config: AndroidLocalizeConfig) -> HashMap<String, String> {
                let mut language_xml: HashMap<_, _> = Default::default();

                for (language, translations) in &self.language_localized {
                    let mut xml = vec![];
                    let mut ordered = translations.clone();

                    ordered.sort_by(|a, b| a.key.cmp(&b.key));

                    for translation in ordered {
                        let content = match &translation.translation {
                            Translation::Localization(localization) => {
                                format!("<string name=\"{}\">{}</string>", translation.sanitize_key_for_android(), localization.sanitize_for_android())
                            }
                            Translation::PluralVariation(plural) => {
                                let mut temp = vec![
                                    format!("<plurals name=\"{}\">", translation.sanitize_key_for_android())
                                ];

                                for single_plural in plural {
                                    temp.push(format!("<item quantity=\"{}\">{}</item>", single_plural.variate.android_key(), single_plural.translation_value.sanitize_for_android()));
                                }

                                temp.push("</plurals>".to_string());

                                temp.join("\n")
                            }
                        };

                        xml.push(content);
                    }

                    if !config.shared_app_name.is_empty() {
                        xml.insert(0, format!("<string name=\"app_name\">{}</string>", config.shared_app_name));
                    }

                    language_xml.insert(language.to_string(), format!("<resources>\n{}\n</resources>", xml.join("\n")));
                }

                language_xml
            }
        }

        #[derive(Debug, Clone, Default)]
        pub struct LocalizationValue {
            pub language_translation: HashMap<String, Translation>,
        }

        #[derive(Debug, Clone)]
        pub struct SinglePluralVariation {
            pub variate: PluralVariate,
            pub translation_value: TranslationValue,
        }

        #[derive(Debug, Clone)]
        pub enum Translation {
            Localization(TranslationValue),
            PluralVariation(Vec<SinglePluralVariation>),
        }

        #[derive(Debug, Clone, Eq, Hash, PartialEq)]
        pub enum PluralVariate {
            One,
            Other,
        }

        impl PluralVariate {
            pub fn android_key(&self) -> &'static str {
                match self {
                    PluralVariate::One => "one",
                    PluralVariate::Other => "other",
                }
            }
        }

        #[derive(Debug, Clone)]
        pub struct TranslationValue {
            pub raw: String,
        }

        impl TranslationValue {
            pub fn sanitize_for_android(&self) -> String {
                self.raw.replace("'", "\\'").replace("$lld", "$d")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::env::current_dir;
    use std::io::Write;
    use super::*;

    // Uncomment to update
    //#[test]
    fn update_android_xmls() {
        let raw = include_bytes!("../test_resources/Localizable.xcstrings");
        let android_actual = parse_from_bytes(raw).localized_per_language().localized_for_android(Default::default());
        let current = current_dir().unwrap().join("test_resources");

        for (language, value) in android_actual {
            let expect = if &language == "en" {
                current.join("android_xml_en.xml")
            } else {
                assert_eq!(language, "nl");

                current.join("android_xml_nl.xml")
            };

            write!(std::fs::File::create(expect).unwrap(), "{}", value).unwrap();
        }
    }

    #[test]
    fn it_works() {
        let raw = include_bytes!("../test_resources/Localizable.xcstrings");
        let android_expected_en = include_bytes!("../test_resources/android_xml_en.xml");
        let android_expected_nl = include_bytes!("../test_resources/android_xml_nl.xml");
        let android_actual = parse_from_bytes(raw).localized_per_language().localized_for_android(Default::default());

        for (language, value) in android_actual { 
            let expect = if &language == "en" {
                android_expected_en.to_vec()
            } else {
                assert_eq!(language, "nl");

                android_expected_nl.to_vec()
            };

            let expect = String::from_utf8(expect).unwrap();
            
            assert_eq!(value.trim(), expect.trim());
        }
    }
}
