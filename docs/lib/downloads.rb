# Repository files served at /assets/<name> as downloads. Propshaft owns /assets, so this runs as middleware in front of it; the route in
# config/routes.rb lets Kiln find the paths.
class Downloads
  FILES = {
    "parcela-skill.txt" => ".agents/skills/parcela/SKILL.md",
    "logo.png" => "assets/brand/logo.png",
    "third-party-notices.txt" => "docs/THIRD_PARTY_NOTICES.txt"
  }.freeze

  def initialize(app = nil, repository: Pathname(__dir__).join("../..").expand_path)
    @app = app
    @repository = repository
  end

  # Copied to the site root by config.files in a build; served from here in development.
  ROOT = { "/install.sh" => "docs/install.sh", "/schema/v1.json" => "docs/schema/v1.json" }.freeze

  def call(env)
    path = env["PATH_INFO"]
    name = File.basename(path)
    source = path.start_with?("/assets/") ? FILES[path.delete_prefix("/assets/")] : ROOT[path]
    return @app.call(env) if source.nil? && @app
    return [ 404, { "content-type" => "text/plain" }, [ "Not found" ] ] unless source

    type = { ".png" => "image/png", ".json" => "application/json", ".sh" => "text/x-shellscript; charset=utf-8" }.fetch(File.extname(name), "text/plain; charset=utf-8")
    [ 200, { "content-type" => type }, [ @repository.join(source).binread ] ]
  end
end
