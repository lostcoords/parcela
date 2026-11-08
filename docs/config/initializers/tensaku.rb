# Tensaku edits page text in development, from the source positions Kiln writes on Markdown
# blocks. The pages that render Markdown files from outside this app may write back to those
# files, and only to them.
if defined?(Tensaku::Config)
  Kiln::Content.source_positions = Rails.env.development?

  Tensaku.configure do |config|
    config.writable_paths.push("README.md", "../CONTRIBUTING.md", "../.agents/skills/parcela/SKILL.md")
  end
end
