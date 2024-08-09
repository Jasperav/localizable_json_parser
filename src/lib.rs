use serde::{Serialize, Serializer};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use crate::types::output::ParsedResult;

pub fn parse_from_string(raw: String) -> ParsedResult {
    parse::from_string(raw)
}

pub fn parse_from_bytes(raw: &[u8]) -> ParsedResult {
    parse_from_string(String::from_utf8(raw.to_vec())?)
}

pub fn parse_from_dir(dir: &PathBuf) -> ParsedResult {
    parse_from_string(std::fs::read_to_string(dir)?)
}

/// https://stackoverflow.com/a/42723390/7715250
/// For use with serde's [serialize_with] attribute
fn ordered_map<S, K: Ord + Serialize, V: Serialize>(
    value: &HashMap<K, V>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let ordered: BTreeMap<_, _> = value.iter().collect();
    ordered.serialize(serializer)
}

mod parse {
    use crate::types::input::{Translation, TranslationTypeContainer};
    use crate::types::output::{
        Localizable, LocalizationValue, Parsed, ParsedResult, PluralVariate, SinglePluralVariation,
        SingleTranslation,
    };

    pub(crate) fn from_string(translations: String) -> ParsedResult {
        let translation: Translation = serde_json::from_str(&translations)?;
        let mut localizable = Localizable::default();

        for (key, language) in &translation.strings {
            let mut localization_value = LocalizationValue::default();

            for (language, translation_type_container) in &language.localizations {
                let translation = match translation_type_container {
                    TranslationTypeContainer::StringUnit(su) => {
                        crate::types::output::Translation::Localization(su.string_unit.clone())
                    }
                    TranslationTypeContainer::Variation(container) => {
                        let v = &container.variations;
                        let mut variations: Vec<_> = Default::default();

                        macro_rules! extract {
                            ($field: expr, $variate: expr) => {
                                if let Some(o) = &$field {
                                    variations.push(SinglePluralVariation {
                                        variate: $variate,
                                        translation_value: o.string_unit.clone(),
                                    });
                                }
                            };
                        }

                        extract!(v.plural.one, PluralVariate::One);
                        extract!(v.plural.two, PluralVariate::Two);
                        extract!(v.plural.few, PluralVariate::Few);
                        extract!(v.plural.many, PluralVariate::Many);
                        extract!(v.plural.other, PluralVariate::Other);

                        crate::types::output::Translation::PluralVariation(variations)
                    }
                };

                localization_value
                    .language_translation
                    .insert(language.to_string(), translation);
            }

            if localization_value.language_translation.get("en").is_none() {
                panic!("The key is the translation, this is not supported because it makes parsing difficult. Key: {key}");
            }

            assert!(
                !key.contains(' '),
                "Keys should not contain whitespaces: {key}"
            );

            localizable.single_translation.push(SingleTranslation {
                key: key.to_string(),
                localization_value,
            });
        }

        localizable
            .single_translation
            .sort_by(|a, b| a.key.cmp(&b.key));

        Ok(Parsed {
            localizable,
            translation,
        })
    }
}

pub mod types {
    pub mod inoutoutput {
        use serde::{Deserialize, Serialize};

        #[derive(Serialize, Debug, Deserialize, Clone, Default)]
        pub struct TranslationValue {
            pub value: String,
            pub state: String,
        }

        #[derive(Debug, Serialize, Deserialize, Clone, Default)]
        pub struct Variation {
            pub plural: Plural,
        }

        #[derive(Serialize, Deserialize, Debug, Clone, Default)]
        pub struct StringUnitContainer {
            #[serde(rename = "stringUnit")]
            pub string_unit: TranslationValue,
        }

        #[derive(Debug, Serialize, Deserialize, Clone, Default)]
        pub struct Plural {
            #[serde(skip_serializing_if = "Option::is_none")]
            pub one: Option<StringUnitContainer>,
            #[serde(skip_serializing_if = "Option::is_none")]
            pub two: Option<StringUnitContainer>,
            #[serde(skip_serializing_if = "Option::is_none")]
            pub other: Option<StringUnitContainer>,
            #[serde(skip_serializing_if = "Option::is_none")]
            pub many: Option<StringUnitContainer>,
            #[serde(skip_serializing_if = "Option::is_none")]
            pub few: Option<StringUnitContainer>,
        }
    }
    pub mod input {
        use std::collections::HashMap;

        use crate::ordered_map;
        use crate::types::inoutoutput::{StringUnitContainer, TranslationValue, Variation};
        use serde::{Deserialize, Serialize};

        #[derive(Debug, Serialize, Deserialize, Clone)]
        pub struct Translation {
            #[serde(serialize_with = "ordered_map")]
            pub strings: HashMap<String, Language>,
            #[allow(dead_code)]
            pub version: String,
            #[allow(dead_code)]
            #[serde(rename = "sourceLanguage")]
            pub source_language: String,
        }

        #[derive(Debug, Serialize, Deserialize, Clone)]
        #[serde(untagged)]
        pub enum TranslationType {
            StringUnit(TranslationValue),
            Variation(Variation),
        }

        #[derive(Debug, Serialize, Deserialize, Clone, Default)]
        pub struct VariationContainer {
            pub variations: Variation,
        }

        #[derive(Debug, Serialize, Deserialize, Clone)]
        #[serde(untagged)]
        pub enum TranslationTypeContainer {
            StringUnit(StringUnitContainer),
            Variation(VariationContainer),
        }

        #[derive(Debug, Serialize, Deserialize, Clone)]
        pub struct Language {
            #[serde(serialize_with = "ordered_map")]
            pub localizations: HashMap<String, TranslationTypeContainer>,
        }
    }

    pub mod output {
        use crate::types::inoutoutput::TranslationValue;
        use enum_const_value::EnumConstValue;

        use std::collections::BTreeMap;
        use std::error::Error;
        use std::fmt::{Display, Formatter};
        use std::path::PathBuf;
        use std::string::FromUtf8Error;

        #[derive(Clone, Debug)]
        pub enum ParsedError {
            ParseToJson(String),
            InvalidUtf8(String),
            Io(String),
        }

        impl Display for ParsedError {
            fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
                match self {
                    ParsedError::ParseToJson(error) => {
                        write!(f, "Invalid XCStrings file: {}", error)
                    }
                    ParsedError::InvalidUtf8(error) => write!(f, "Invalid UTF8: {}", error),
                    ParsedError::Io(error) => write!(f, "IO error: {}", error),
                }
            }
        }

        impl Error for ParsedError {
            fn source(&self) -> Option<&(dyn Error + 'static)> {
                None
            }
        }

        impl From<FromUtf8Error> for ParsedError {
            fn from(value: FromUtf8Error) -> Self {
                ParsedError::InvalidUtf8(value.to_string())
            }
        }

        impl From<std::io::Error> for ParsedError {
            fn from(value: std::io::Error) -> Self {
                ParsedError::Io(value.to_string())
            }
        }

        impl From<serde_json::Error> for ParsedError {
            fn from(value: serde_json::Error) -> Self {
                ParsedError::ParseToJson(value.to_string())
            }
        }

        pub type ParsedResult = Result<Parsed, ParsedError>;

        #[derive(Debug, Clone)]
        pub struct Parsed {
            pub localizable: Localizable,
            pub translation: super::input::Translation,
        }

        #[derive(Debug, Clone, Default)]
        pub struct Localizable {
            pub single_translation: Vec<SingleTranslation>,
        }

        #[derive(Debug, Clone)]
        pub struct SingleTranslation {
            pub key: String,
            pub localization_value: LocalizationValue,
        }

        #[derive(Debug, Clone, Default)]
        pub struct LocalizedPerLanguageInfo {
            pub word_count: usize,
            pub translations: Vec<SingleLocalizedPerLanguage>,
        }

        #[derive(Debug, Clone)]
        pub struct SingleLocalizedPerLanguage {
            pub key: String,
            pub translation: Translation,
        }

        impl SingleLocalizedPerLanguage {
            pub fn sanitize_key_for_android(&self) -> String {
                self.key.replace('-', "_")
            }
        }

        #[derive(Debug, Clone, Default)]
        pub struct LocalizedPerLanguage {
            pub language_localized: BTreeMap<String, LocalizedPerLanguageInfo>,
        }

        impl Localizable {
            pub fn localized_per_language(&self) -> LocalizedPerLanguage {
                let mut localized_per_language = LocalizedPerLanguage::default();

                for single_translation in &self.single_translation {
                    for (language, translation) in
                        &single_translation.localization_value.language_translation
                    {
                        let single_localized_per_language = localized_per_language
                            .language_localized
                            .entry(language.to_string())
                            .or_default();

                        single_localized_per_language.translations.push(
                            SingleLocalizedPerLanguage {
                                key: single_translation.key.to_string(),
                                translation: translation.clone(),
                            },
                        );

                        let length = match translation {
                            Translation::Localization(l) => words_count::count(&l.value).words,
                            Translation::PluralVariation(pv) => pv
                                .iter()
                                .map(|single| {
                                    words_count::count(&single.translation_value.value).words
                                })
                                .sum(),
                        };

                        single_localized_per_language.word_count += length;
                    }
                }

                for (language, localized) in &localized_per_language.language_localized {
                    log::debug!(
                        "Language: {} word count: {}",
                        language, localized.word_count
                    )
                }

                localized_per_language
            }
        }

        #[derive(Debug, Clone, Default)]
        pub struct AndroidLocalizeConfig {
            pub shared_app_name: String,
            pub res_path_to_overwrite_xmls: Option<PathBuf>,
        }

        impl LocalizedPerLanguage {
            pub fn localized_for_android(
                &self,
                config: AndroidLocalizeConfig,
            ) -> Result<BTreeMap<String, String>, ParsedError> {
                let mut language_xml: BTreeMap<_, _> = Default::default();

                for (language, translations) in &self.language_localized {
                    let mut xml = vec![];
                    let ordered = translations.clone();

                    for translation in ordered.translations {
                        let content = match &translation.translation {
                            Translation::Localization(localization) => {
                                format!(
                                    "<string name=\"{}\">{}</string>",
                                    translation.sanitize_key_for_android(),
                                    localization.sanitize_for_android()
                                )
                            }
                            Translation::PluralVariation(plural) => {
                                let mut temp = vec![format!(
                                    "<plurals name=\"{}\">",
                                    translation.sanitize_key_for_android()
                                )];

                                for single_plural in plural {
                                    temp.push(format!(
                                        "<item quantity=\"{}\">{}</item>",
                                        single_plural.variate.android_key(),
                                        single_plural.translation_value.sanitize_for_android()
                                    ));
                                }

                                temp.push("</plurals>".to_string());

                                temp.join("\n")
                            }
                        };

                        xml.push(content);
                    }

                    if !config.shared_app_name.is_empty() {
                        xml.insert(
                            0,
                            format!(
                                "<string name=\"app_name\">{}</string>",
                                config.shared_app_name
                            ),
                        );
                    }

                    language_xml.insert(
                        language.to_string(),
                        format!("<resources>\n{}\n</resources>", xml.join("\n")),
                    );
                }

                if let Some(path) = config.res_path_to_overwrite_xmls {
                    for (language, content) in &language_xml {
                        let suffix_dir = if language == "en" {
                            "".to_string()
                        } else {
                            format!("-{language}")
                        };

                        let path_to_file =
                            path.join(format!("values{suffix_dir}")).join("strings.xml");

                        assert!(
                            path_to_file.exists(),
                            "Path does not exists: {:#?}",
                            path_to_file
                        );

                        std::fs::write(path_to_file, content)?;
                    }
                }

                Ok(language_xml)
            }
        }

        #[derive(Debug, Clone, Default)]
        pub struct LocalizationValue {
            pub language_translation: BTreeMap<String, Translation>,
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

        impl Translation {
            pub fn expect_localization(self) -> TranslationValue {
                match self {
                    Translation::Localization(tv) => tv,
                    _ => panic!(),
                }
            }

            pub fn expect_plural_variation(self) -> Vec<SinglePluralVariation> {
                match self {
                    Translation::PluralVariation(pv) => pv,
                    _ => panic!(),
                }
            }
        }

        #[derive(Debug, Clone, Eq, Hash, PartialEq, EnumConstValue)]
        pub enum PluralVariate {
            One,
            Two,
            Few,
            Many,
            Other,
        }

        impl PluralVariate {
            pub fn from_android_key(str: &str) -> Option<Self> {
                PluralVariate::all_values()
                    .into_iter()
                    .find(|variate| variate.android_key() == str)
            }

            pub fn android_key(&self) -> &'static str {
                match self {
                    PluralVariate::One => "one",
                    PluralVariate::Two => "two",
                    PluralVariate::Few => "few",
                    PluralVariate::Many => "many",
                    PluralVariate::Other => "other",
                }
            }
        }

        impl TranslationValue {
            pub fn sanitize_for_android(&self) -> String {
                self.value.replace('\'', "\\'").replace("$lld", "$d")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::output::AndroidLocalizeConfig;
    use std::env::current_dir;

    // Uncomment to update
    #[test]
    fn update_android_xmls() {
        let raw = include_bytes!("../test_resources/Localizable.xcstrings");
        let current = current_dir().unwrap().join("test_resources");
        let _ = parse_from_bytes(raw)
            .unwrap()
            .localizable
            .localized_per_language()
            .localized_for_android(AndroidLocalizeConfig {
                res_path_to_overwrite_xmls: Some(current),
                ..Default::default()
            });
    }

    #[test]
    fn it_works() {
        let raw = include_bytes!("../test_resources/Localizable.xcstrings");
        let android_expected_en = include_bytes!("../test_resources/values/strings.xml");
        let android_expected_nl = include_bytes!("../test_resources/values-nl/strings.xml");
        let parsed = parse_from_bytes(raw).unwrap();
        let localized_per_language = parsed.localizable.localized_per_language();
        let android_actual = localized_per_language
            .localized_for_android(Default::default())
            .unwrap();

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
