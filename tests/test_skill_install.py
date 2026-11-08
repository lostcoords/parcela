import os
import re
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path


DOCUMENT = Path(__file__).resolve().parents[1] / 'docs/app/views/pages/worktrees.html.md'
PAYLOAD = b'---\nname: parcela\ndescription: Test fixture\n---\nFixture instructions.\n'
ZSH = shutil.which('zsh')


class ClaudeSkillInstallation(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='parcela-skill-install-')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.project = self.root / 'project with spaces'
        self.project.mkdir()
        self.home = self.root / 'home'
        self.home.mkdir()
        self.state = self.root / 'state'
        self.state.mkdir()
        (self.state / 'sentinel').write_bytes(b'unchanged state')
        self.bin = self.root / 'bin'
        self.bin.mkdir()
        command = self.bin / 'parcela'
        command.write_text("#!/bin/sh\ntest \"$#\" -eq 1 && test \"$1\" = --skill || exit 2\n"
                           "printf '%s' '" + PAYLOAD.decode() + "'\n")
        command.chmod(0o700)
        self.environment = {
            'PATH': str(self.bin) + os.pathsep + os.defpath,
            'HOME': str(self.home),
            'PARCELA_STATE_DIR': str(self.state),
        }
        section = DOCUMENT.read_text().split('### Install the skill for Claude Code\n', 1)[1]
        self.script = re.search(r'```sh\n(.*?)\n```', section, re.S).group(1)

    def install(self, script=None, shell='/bin/sh'):
        command = [shell, '-f'] if Path(shell).name == 'zsh' else [shell]
        result = subprocess.run(
            [*command, '-c', self.script if script is None else script],
            cwd=self.project, env=self.environment, capture_output=True,
            text=True, timeout=5)
        self.assertEqual((self.state / 'sentinel').read_bytes(), b'unchanged state')
        self.assertEqual(list(self.state.iterdir()), [self.state / 'sentinel'])
        self.assertEqual(list(self.root.rglob('.install.*')), [])
        return result

    def refuse_redirected_ancestor(self, ancestor):
        external = self.home / '.claude'
        external.mkdir()
        redirected = self.project / ancestor
        redirected.parent.mkdir(parents=True, exist_ok=True)
        redirected.symlink_to(external, target_is_directory=True)
        result = self.install()
        self.assertNotEqual(result.returncode, 0,
                            'project installation followed an ancestor symlink into personal scope')
        self.assertEqual(list(external.iterdir()), [])
        self.assertTrue(redirected.is_symlink())
        self.assertEqual(redirected.readlink(), external)

    def test_project_root_symlink_cannot_install_a_personal_skill(self):
        self.refuse_redirected_ancestor('.claude')

    def test_skills_directory_symlink_cannot_redirect_the_installation(self):
        self.refuse_redirected_ancestor('.claude/skills')

    def test_skill_directory_symlink_cannot_redirect_the_installation(self):
        self.refuse_redirected_ancestor('.claude/skills/parcela')

    def test_dangling_ancestor_symlink_is_preserved_without_installation(self):
        for index, ancestor in enumerate(('.claude', '.claude/skills', '.claude/skills/parcela')):
            with self.subTest(ancestor=ancestor):
                project = self.root / ('dangling-' + str(index))
                project.mkdir()
                self.project = project
                redirected = project / ancestor
                redirected.parent.mkdir(parents=True, exist_ok=True)
                missing = self.home / ('absent-' + str(index))
                redirected.symlink_to(missing, target_is_directory=True)
                result = self.install()
                self.assertNotEqual(result.returncode, 0)
                self.assertTrue(redirected.is_symlink())
                self.assertEqual(redirected.readlink(), missing)
                self.assertFalse(missing.exists())

    def test_fresh_project_installs_exact_export_only_in_project_scope(self):
        result = self.install()
        self.assertEqual(result.returncode, 0, result.stderr)
        target = self.project / '.claude/skills/parcela/SKILL.md'
        self.assertEqual(target.read_bytes(), PAYLOAD)
        self.assertFalse((self.home / '.claude').exists())

    def test_existing_custom_skill_is_not_replaced(self):
        target = self.project / '.claude/skills/parcela/SKILL.md'
        target.parent.mkdir(parents=True)
        target.write_bytes(b'custom instructions')
        result = self.install()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(target.read_bytes(), b'custom instructions')

    def test_existing_directory_is_not_used_as_a_publication_target(self):
        target = self.project / '.claude/skills/parcela/SKILL.md'
        target.mkdir(parents=True)
        result = self.install()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(list(target.iterdir()), [])

    def test_existing_broken_skill_symlink_is_not_replaced(self):
        target = self.project / '.claude/skills/parcela/SKILL.md'
        target.parent.mkdir(parents=True)
        target.symlink_to('absent')
        result = self.install()
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue(target.is_symlink())
        self.assertEqual(target.readlink(), Path('absent'))

    def test_empty_or_failed_export_never_publishes_a_skill(self):
        bodies = ('exit 0', 'exit 2', "printf 'partial export'; exit 2")
        for index, body in enumerate(bodies):
            with self.subTest(body=body):
                self.project = self.root / ('failed-export-' + str(index))
                self.project.mkdir()
                (self.bin / 'parcela').write_text('#!/bin/sh\n' + body + '\n')
                result = self.install()
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse((self.project / '.claude/skills/parcela/SKILL.md').exists())

    def test_explicit_personal_scope_installs_only_under_fake_home(self):
        text = DOCUMENT.read_text()
        assignment = re.search(r'line with `(skill_root=[^`]+)`', text).group(1)
        script = self.script.replace('skill_root=.claude', assignment)
        result = self.install(script)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.home / '.claude/skills/parcela/SKILL.md').read_bytes(), PAYLOAD)
        self.assertFalse((self.project / '.claude').exists())

    @unittest.skipUnless(ZSH, 'zsh is not installed')
    def test_zsh_can_paste_the_exact_project_install_block(self):
        result = self.install(shell=ZSH)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.project / '.claude/skills/parcela/SKILL.md').read_bytes(), PAYLOAD)
        self.assertFalse((self.home / '.claude').exists())

    @unittest.skipUnless(ZSH, 'zsh is not installed')
    def test_zsh_can_paste_the_explicit_personal_install_block(self):
        assignment = re.search(r'line with `(skill_root=[^`]+)`', DOCUMENT.read_text()).group(1)
        script = self.script.replace('skill_root=.claude', assignment)
        result = self.install(script, shell=ZSH)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.home / '.claude/skills/parcela/SKILL.md').read_bytes(), PAYLOAD)
        self.assertFalse((self.project / '.claude').exists())


if __name__ == '__main__':
    unittest.main()
