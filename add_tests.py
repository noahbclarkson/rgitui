#!/usr/bin/env python3
import sys

TAG_TESTS = b'''
#[cfg(test)]
mod tests {
    use super::*;

    // ========================
    // Invalid cases
    // ========================

    #[test]
    fn test_validate_tag_name_empty() {
        let result = TagDialog::validate_tag_name("");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Tag name cannot be empty");
    }

    #[test]
    fn test_validate_tag_name_contains_space() {
        let result = TagDialog::validate_tag_name("v1.0 release");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Tag name cannot contain spaces");
    }

    #[test]
    fn test_validate_tag_name_starts_with_dot() {
        let result = TagDialog::validate_tag_name(".tag");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Tag name cannot start with '.' or '-'");
    }

    #[test]
    fn test_validate_tag_name_starts_with_hyphen() {
        let result = TagDialog::validate_tag_name("-v1");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Tag name cannot start with '.' or '-'");
    }

    #[test]
    fn test_validate_tag_name_ends_with_dot() {
        let result = TagDialog::validate_tag_name("v1.");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Tag name cannot end with '.' or '/'");
    }

    #[test]
    fn test_validate_tag_name_ends_with_slash() {
        let result = TagDialog::validate_tag_name("v1/");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Tag name cannot end with '.' or '/'");
    }

    #[test]
    fn test_validate_tag_name_contains_double_dot() {
        let result = TagDialog::validate_tag_name("v1..0");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Tag name cannot contain '..'");
    }

    #[test]
    fn test_validate_tag_name_contains_tilde() {
        let result = TagDialog::validate_tag_name("v1~2");
        assert!(result.is_some());
        assert_eq!(
            result.unwrap(),
            "Tag name cannot contain '~', '^', ':', or '\\\\'"
        );
    }

    #[test]
    fn test_validate_tag_name_contains_caret() {
        let result = TagDialog::validate_tag_name("v1^2");
        assert!(result.is_some());
        assert_eq!(
            result.unwrap(),
            "Tag name cannot contain '~', '^', ':', or '\\\\'"
        );
    }

    #[test]
    fn test_validate_tag_name_contains_colon() {
        let result = TagDialog::validate_tag_name("v1:2");
        assert!(result.is_some());
        assert_eq!(
            result.unwrap(),
            "Tag name cannot contain '~', '^', ':', or '\\\\'"
        );
    }

    #[test]
    fn test_validate_tag_name_contains_backslash() {
        let result = TagDialog::validate_tag_name("v1\\\\2");
        assert!(result.is_some());
        assert_eq!(
            result.unwrap(),
            "Tag name cannot contain '~', '^', ':', or '\\\\'"
        );
    }

    #[test]
    fn test_validate_tag_name_contains_question_mark() {
        let result = TagDialog::validate_tag_name("v1?2");
        assert!(result.is_some());
        assert_eq!(
            result.unwrap(),
            "Tag name cannot contain glob characters"
        );
    }

    #[test]
    fn test_validate_tag_name_contains_asterisk() {
        let result = TagDialog::validate_tag_name("v1*2");
        assert!(result.is_some());
        assert_eq!(
            result.unwrap(),
            "Tag name cannot contain glob characters"
        );
    }

    #[test]
    fn test_validate_tag_name_contains_bracket() {
        let result = TagDialog::validate_tag_name("v1[2]");
        assert!(result.is_some());
        assert_eq!(
            result.unwrap(),
            "Tag name cannot contain glob characters"
        );
    }

    #[test]
    fn test_validate_tag_name_contains_control_char() {
        let result = TagDialog::validate_tag_name("v1\\x002");
        assert!(result.is_some());
        assert_eq!(
            result.unwrap(),
            "Tag name cannot contain control characters"
        );
    }

    #[test]
    fn test_validate_tag_name_contains_at_brace() {
        let result = TagDialog::validate_tag_name("v1@{2");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Tag name cannot contain '@{'");
    }

    #[test]
    fn test_validate_tag_name_contains_double_slash() {
        let result = TagDialog::validate_tag_name("v1//2");
        assert!(result.is_some());
        assert_eq!(
            result.unwrap(),
            "Tag name cannot contain consecutive slashes"
        );
    }

    #[test]
    fn test_validate_tag_name_ends_with_lock() {
        let result = TagDialog::validate_tag_name("v1.lock");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Tag name cannot end with '.lock'");
    }

    // ========================
    // Valid cases
    // ========================

    #[test]
    fn test_validate_tag_name_valid_simple() {
        assert!(TagDialog::validate_tag_name("v1.0.0").is_none());
    }

    #[test]
    fn test_validate_tag_name_valid_with_v_prefix() {
        assert!(TagDialog::validate_tag_name("v2.0.0-rc1").is_none());
    }

    #[test]
    fn test_validate_tag_name_valid_underscore() {
        assert!(TagDialog::validate_tag_name("v1_0_0").is_none());
    }

    #[test]
    fn test_validate_tag_name_valid_hyphen() {
        assert!(TagDialog::validate_tag_name("release-1.0.0").is_none());
    }

    #[test]
    fn test_validate_tag_name_valid_slash() {
        assert!(TagDialog::validate_tag_name("rgitui/v1.0").is_none());
    }

    #[test]
    fn test_validate_tag_name_single_char() {
        assert!(TagDialog::validate_tag_name("v").is_none());
    }
}
'''

RENAME_TESTS = b'''
#[cfg(test)]
mod tests {
    use super::*;

    // ========================
    // Invalid cases
    // ========================

    #[test]
    fn test_validate_rename_empty() {
        let result = RenameDialog::validate("");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Branch name cannot be empty");
    }

    #[test]
    fn test_validate_rename_contains_space() {
        let result = RenameDialog::validate("foo bar");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Branch name cannot contain spaces");
    }

    #[test]
    fn test_validate_rename_starts_with_dot() {
        let result = RenameDialog::validate(".hidden");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Cannot start with '.' or '-'");
    }

    #[test]
    fn test_validate_rename_starts_with_hyphen() {
        let result = RenameDialog::validate("-foo");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Cannot start with '.' or '-'");
    }

    #[test]
    fn test_validate_rename_ends_with_dot() {
        let result = RenameDialog::validate("foo.");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Cannot end with '.' or '/'");
    }

    #[test]
    fn test_validate_rename_ends_with_slash() {
        let result = RenameDialog::validate("foo/");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Cannot end with '.' or '/'");
    }

    #[test]
    fn test_validate_rename_contains_double_dot() {
        let result = RenameDialog::validate("foo..bar");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Cannot contain '..' or '//'");
    }

    #[test]
    fn test_validate_rename_contains_double_slash() {
        let result = RenameDialog::validate("foo//bar");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Cannot contain '..' or '//'");
    }

    #[test]
    fn test_validate_rename_contains_tilde() {
        let result = RenameDialog::validate("foo~1");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Contains invalid characters");
    }

    #[test]
    fn test_validate_rename_contains_caret() {
        let result = RenameDialog::validate("foo^1");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Contains invalid characters");
    }

    #[test]
    fn test_validate_rename_contains_colon() {
        let result = RenameDialog::validate("foo:bar");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Contains invalid characters");
    }

    #[test]
    fn test_validate_rename_contains_backslash() {
        let result = RenameDialog::validate("foo\\\\bar");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Contains invalid characters");
    }

    #[test]
    fn test_validate_rename_contains_question_mark() {
        let result = RenameDialog::validate("foo?bar");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Contains invalid characters");
    }

    #[test]
    fn test_validate_rename_contains_asterisk() {
        let result = RenameDialog::validate("foo*bar");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Contains invalid characters");
    }

    #[test]
    fn test_validate_rename_contains_bracket() {
        let result = RenameDialog::validate("foo[bar]");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Contains invalid characters");
    }

    #[test]
    fn test_validate_rename_contains_control_char() {
        let result = RenameDialog::validate("foo\\x00bar");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Contains control characters");
    }

    #[test]
    fn test_validate_rename_contains_at_brace() {
        let result = RenameDialog::validate("foo@{bar");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Invalid ref name");
    }

    #[test]
    fn test_validate_rename_is_at() {
        let result = RenameDialog::validate("@");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Invalid ref name");
    }

    #[test]
    fn test_validate_rename_ends_with_lock() {
        let result = RenameDialog::validate("foo.lock");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Cannot end with '.lock'");
    }

    // ========================
    // Valid cases
    // ========================

    #[test]
    fn test_validate_rename_valid_simple() {
        assert!(RenameDialog::validate("main").is_none());
    }

    #[test]
    fn test_validate_rename_valid_feature() {
        assert!(RenameDialog::validate("feature-xyz-123").is_none());
    }

    #[test]
    fn test_validate_rename_valid_underscore() {
        assert!(RenameDialog::validate("feature_xyz_123").is_none());
    }

    #[test]
    fn test_validate_rename_valid_slash() {
        assert!(RenameDialog::validate("feature/xyz").is_none());
    }

    #[test]
    fn test_validate_rename_valid_with_dot() {
        assert!(RenameDialog::validate("v1.0.0").is_none());
    }

    #[test]
    fn test_validate_rename_single_char() {
        assert!(RenameDialog::validate("a").is_none());
    }
}
'''

BRANCH_TESTS = b'''
#[cfg(test)]
mod tests {
    use super::*;

    // ========================
    // Invalid cases
    // ========================

    #[test]
    fn test_validate_branch_name_empty() {
        let result = BranchDialog::validate_branch_name("");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Branch name cannot be empty");
    }

    #[test]
    fn test_validate_branch_name_contains_space() {
        let result = BranchDialog::validate_branch_name("feature branch");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Branch name cannot contain spaces");
    }

    #[test]
    fn test_validate_branch_name_starts_with_dot() {
        let result = BranchDialog::validate_branch_name(".hidden");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Branch name cannot start with '.' or '-'");
    }

    #[test]
    fn test_validate_branch_name_starts_with_hyphen() {
        let result = BranchDialog::validate_branch_name("-foo");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Branch name cannot start with '.' or '-'");
    }

    #[test]
    fn test_validate_branch_name_ends_with_dot() {
        let result = BranchDialog::validate_branch_name("foo.");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Branch name cannot end with '.' or '/'");
    }

    #[test]
    fn test_validate_branch_name_ends_with_slash() {
        let result = BranchDialog::validate_branch_name("foo/");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Branch name cannot end with '.' or '/'");
    }

    #[test]
    fn test_validate_branch_name_contains_double_dot() {
        let result = BranchDialog::validate_branch_name("foo..bar");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Branch name cannot contain '..'");
    }

    #[test]
    fn test_validate_branch_name_contains_tilde() {
        let result = BranchDialog::validate_branch_name("foo~1");
        assert!(result.is_some());
        assert_eq!(
            result.unwrap(),
            "Branch name cannot contain '~', '^', ':', or '\\\\'"
        );
    }

    #[test]
    fn test_validate_branch_name_contains_caret() {
        let result = BranchDialog::validate_branch_name("foo^1");
        assert!(result.is_some());
        assert_eq!(
            result.unwrap(),
            "Branch name cannot contain '~', '^', ':', or '\\\\'"
        );
    }

    #[test]
    fn test_validate_branch_name_contains_colon() {
        let result = BranchDialog::validate_branch_name("foo:bar");
        assert!(result.is_some());
        assert_eq!(
            result.unwrap(),
            "Branch name cannot contain '~', '^', ':', or '\\\\'"
        );
    }

    #[test]
    fn test_validate_branch_name_contains_backslash() {
        let result = BranchDialog::validate_branch_name("foo\\\\bar");
        assert!(result.is_some());
        assert_eq!(
            result.unwrap(),
            "Branch name cannot contain '~', '^', ':', or '\\\\'"
        );
    }

    #[test]
    fn test_validate_branch_name_contains_question_mark() {
        let result = BranchDialog::validate_branch_name("foo?bar");
        assert!(result.is_some());
        assert_eq!(
            result.unwrap(),
            "Branch name cannot contain glob characters"
        );
    }

    #[test]
    fn test_validate_branch_name_contains_asterisk() {
        let result = BranchDialog::validate_branch_name("foo*bar");
        assert!(result.is_some());
        assert_eq!(
            result.unwrap(),
            "Branch name cannot contain glob characters"
        );
    }

    #[test]
    fn test_validate_branch_name_contains_bracket() {
        let result = BranchDialog::validate_branch_name("foo[bar]");
        assert!(result.is_some());
        assert_eq!(
            result.unwrap(),
            "Branch name cannot contain glob characters"
        );
    }

    #[test]
    fn test_validate_branch_name_contains_control_char() {
        let result = BranchDialog::validate_branch_name("foo\\x00bar");
        assert!(result.is_some());
        assert_eq!(
            result.unwrap(),
            "Branch name cannot contain control characters"
        );
    }

    #[test]
    fn test_validate_branch_name_contains_at_brace() {
        let result = BranchDialog::validate_branch_name("foo@{bar");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Branch name cannot contain '@{'");
    }

    #[test]
    fn test_validate_branch_name_is_at() {
        let result = BranchDialog::validate_branch_name("@");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Branch name cannot be '@'");
    }

    #[test]
    fn test_validate_branch_name_contains_double_slash() {
        let result = BranchDialog::validate_branch_name("foo//bar");
        assert!(result.is_some());
        assert_eq!(
            result.unwrap(),
            "Branch name cannot contain consecutive slashes"
        );
    }

    #[test]
    fn test_validate_branch_name_ends_with_lock() {
        let result = BranchDialog::validate_branch_name("foo.lock");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Branch name cannot end with '.lock'");
    }

    // ========================
    // Valid cases
    // ========================

    #[test]
    fn test_validate_branch_name_valid_simple() {
        assert!(BranchDialog::validate_branch_name("main").is_none());
    }

    #[test]
    fn test_validate_branch_name_valid_feature() {
        assert!(BranchDialog::validate_branch_name("feature-xyz-123").is_none());
    }

    #[test]
    fn test_validate_branch_name_valid_underscore() {
        assert!(BranchDialog::validate_branch_name("feature_xyz_123").is_none());
    }

    #[test]
    fn test_validate_branch_name_valid_slash() {
        assert!(BranchDialog::validate_branch_name("feature/xyz").is_none());
    }

    #[test]
    fn test_validate_branch_name_valid_with_dot() {
        assert!(BranchDialog::validate_branch_name("v1.0.0").is_none());
    }

    #[test]
    fn test_validate_branch_name_valid_long() {
        let name = "feature/rgitui/refactor/blame-view-performance";
        assert!(BranchDialog::validate_branch_name(name).is_none());
    }

    #[test]
    fn test_validate_branch_name_single_char() {
        assert!(BranchDialog::validate_branch_name("a").is_none());
    }
}
'''

FILES = [
    ('crates/rgitui_workspace/src/tag_dialog.rs', TAG_TESTS),
    ('crates/rgitui_workspace/src/rename_dialog.rs', RENAME_TESTS),
    ('crates/rgitui_workspace/src/branch_dialog.rs', BRANCH_TESTS),
]

for fname, tests in FILES:
    with open(fname, 'rb') as f:
        content = f.read()
    
    # Structure: file ends with 't()\n    }\n}\n'
    # content[-8:] = '    }\n}\n' (8 bytes)
    # content[-4:] = '}\n}\n' (4 bytes)
    # content[:-8] = everything before the '    }\n}\n'
    
    assert content[-8:] == b'    }\n}\n', fname + ': last 8 bytes wrong: ' + repr(content[-8:])
    assert content[-4:] == b'}\n}\n', fname + ': last 4 bytes wrong: ' + repr(content[-4:])
    
    new_content = content[:-8] + tests + content[-4:]
    
    with open(fname, 'wb') as f:
        f.write(new_content)
    
    verify = open(fname, 'rb').read()
    assert verify.endswith(b'}\n}\n'), fname + ': bad ending: ' + repr(verify[-10:])
    assert b'#[cfg(test)]' in verify, fname + ': test module not found'
    print(fname + ': OK (' + str(len(verify)) + ' bytes)')
