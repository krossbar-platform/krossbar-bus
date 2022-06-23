use std::collections::HashSet;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum NameError {
    #[error("Invalid name section pattern. Can be `*`, `**`, or string. Found `{0}`")]
    InvalidSection(String),
    #[error("Service names can't contain asterisks")]
    NameContainsAsterisk,
    #[error("Empty name section. Sections between dots should contain at least one characer")]
    EmptySection,
}

enum NameSectionPattern {
    /// `*`
    AnyName,
    /// `**`
    AnySections,
    /// {name}
    Section(String),
}

impl NameSectionPattern {
    pub fn from_string(string: &str) -> Result<Self, NameError> {
        if string.is_empty() {
            return Err(NameError::EmptySection);
        }

        if string.contains('*') {
            return if string == "*" {
                Ok(Self::AnyName)
            } else if string == "**" {
                Ok(Self::AnySections)
            } else {
                Err(NameError::InvalidSection(string.into()))
            };
        }

        Ok(Self::Section(string.into()))
    }
}

pub struct NamePattern {
    section_patterns: Vec<NameSectionPattern>,
}

impl NamePattern {
    pub fn from_string(string: &str) -> Result<Self, NameError> {
        let section_strings: Vec<&str> = string.split('.').collect();

        let mut this = Self {
            section_patterns: vec![],
        };

        for section_string in section_strings {
            this.section_patterns
                .push(NameSectionPattern::from_string(section_string)?);
        }

        Ok(this)
    }

    pub fn matches(&self, service_name: &str) -> Result<bool, NameError> {
        if service_name.contains('*') {
            return Err(NameError::NameContainsAsterisk);
        }

        let name_sections: Vec<&str> = service_name.split(".").collect();

        if name_sections.contains(&"") {
            return Err(NameError::EmptySection);
        }

        // Well, this is classic pattern-matching algorithm. We walk by segments
        // and try to match a segment against a pattern. If matches, we push index into a match
        // tree. If match tree depth equals to a number of patterns at the end of mathing,
        // it means whole string matches. *pattern_indices* contains a set of tree leaves
        let mut pattern_tree_leaves = HashSet::new();
        // Starting from the first segment
        pattern_tree_leaves.insert(0);

        for name_section in name_sections.iter() {
            // Next iteration pattern tree leafs
            let mut next_pattern_tree_leaves = HashSet::new();

            // And try to match current section against leaf patterns
            for index in pattern_tree_leaves {
                // If patern index is out of section patterns bound, we've
                // already ran out of patterns, but still have name sections.
                // Let's hope we still have indices in the middle of section patterns
                if index == self.section_patterns.len() {
                    continue;
                }

                match self.section_patterns[index] {
                    // This consumes pattern section, and matches any section
                    NameSectionPattern::AnyName => {
                        next_pattern_tree_leaves.insert(index + 1);
                    }
                    // This pattern add another segment to the tree leaves,
                    // but keep current too, because following segments will match
                    // `**` pattern
                    NameSectionPattern::AnySections => {
                        next_pattern_tree_leaves.insert(index);
                        next_pattern_tree_leaves.insert(index + 1);
                    }
                    // Consumes pattern section and if matches, shifts to the next pattern
                    NameSectionPattern::Section(ref pattern_section) => {
                        if pattern_section == *name_section {
                            next_pattern_tree_leaves.insert(index + 1);
                        }
                    }
                }
            }

            // Update pattern tree leaves for the next name sections
            pattern_tree_leaves = next_pattern_tree_leaves;
        }

        // String matches only if after iterating over all name sections, we have
        // matched all patterns
        Ok(pattern_tree_leaves.contains(&self.section_patterns.len()))
    }
}
