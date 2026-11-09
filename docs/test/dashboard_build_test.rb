require "test_helper"

# The dashboard's page and CSS are committed and compiled into the Parcela binary.
class DashboardBuildTest < ActiveSupport::TestCase
  ROOT = Rails.root.join("..")
  STALE = "Run scripts/build-dashboard"

  test "the committed pages match their sources" do
    Dir.mktmpdir do |dir|
      begin
        ENV["DASHBOARD_OUT"] = dir
        load ROOT.join("scripts/dashboard/render.rb").to_s
      ensure
        ENV.delete("DASHBOARD_OUT")
      end
      %w[index.html logs.html].each do |file|
        assert_equal ROOT.join("src/dashboard", file).read, File.read(File.join(dir, file)), STALE
      end
    end
  end

  test "the committed CSS matches its source" do
    Dir.mktmpdir do |dir|
      output = File.join(dir, "ui.css")
      Dir.chdir(Rails.root) do
        assert system("bin/rails", "tailwindcss:engines", out: File::NULL)
        assert system("bundle", "exec", "tailwindcss", "-i", ROOT.join("scripts/dashboard/ui.input.css").to_s, "-o", output, "--minify", out: File::NULL, err: File::NULL)
      end
      assert_equal ROOT.join("src/dashboard/ui.css").read, File.read(output), STALE
    end
  end
end
