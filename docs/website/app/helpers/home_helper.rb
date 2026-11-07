# Data and small pieces for the landing page.
module HomeHelper
  INSTALL = "curl -fsSL https://parcela.run/install.sh | sh".freeze
  INSTALL_PARTS = [ [ "#B392F0", "curl" ], [ "#79B8FF", " -fsSL" ], [ "#9ECBFF", " https://parcela.run/install.sh" ], [ "#F97583", " |" ], [ "#B392F0", " sh" ] ].freeze

  # Brand marks come from Simple Icons (CC0) through config/brands.json. The hero shows agents
  # in their own colors; nil keeps the text color.
  AGENT_COLORS = { "Claude Code" => "#d97757", "Codex" => "#10a37f", "Copilot" => "#8957e5", "Gemini CLI" => "#4796e3" }.freeze
  AGENT_GRADIENTS = { "Gemini CLI" => "linear-gradient(90deg, #4796e3, #9177c7, #ca6673)" }.freeze
  AGENT_NAMES = [ "Claude Code", "Codex", "Cursor", "Copilot", "Gemini CLI", "OpenCode" ].freeze
  FRAMEWORK_NAMES = %w[Rails Django Next.js Vite Phoenix Laravel].freeze

  GITHUB = "M12 .297c-6.63 0-12 5.373-12 12c0 5.303 3.438 9.8 8.205 11.385c.6.113.82-.258.82-.577c0-.285-.01-1.04-.015-2.04c-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729c1.205.084 1.838 1.236 1.838 1.236c1.07 1.835 2.809 1.305 3.495.998c.108-.776.417-1.305.76-1.605c-2.665-.3-5.466-1.332-5.466-5.93c0-1.31.465-2.38 1.235-3.22c-.135-.303-.54-1.523.105-3.176c0 0 1.005-.322 3.3 1.23c.96-.267 1.98-.399 3-.405c1.02.006 2.04.138 3 .405c2.28-1.552 3.285-1.23 3.285-1.23c.645 1.653.24 2.873.12 3.176c.765.84 1.23 1.91 1.23 3.22c0 4.61-2.805 5.625-5.475 5.92c.42.36.81 1.096.81 2.22c0 1.606-.015 2.896-.015 3.286c0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12".freeze

  # Lucide icons (ISC), as the inner markup of a 24px stroked SVG.
  LUCIDE = {
    "arrow-right" => '<path d="M5 12h14"/><path d="m12 5 7 7-7 7"/>',
    "arrow-up-right" => '<path d="M7 7h10v10"/><path d="M7 17 17 7"/>',
    "tag" => '<path d="M12.586 2.586A2 2 0 0 0 11.172 2H4a2 2 0 0 0-2 2v7.172a2 2 0 0 0 .586 1.414l8.704 8.704a2.426 2.426 0 0 0 3.42 0l6.58-6.58a2.426 2.426 0 0 0 0-3.42z"/><circle cx="7.5" cy="7.5" r=".5" fill="currentColor"/>',
    "check" => '<path d="M20 6 9 17l-5-5"/>',
    "x" => '<path d="M18 6 6 18"/><path d="m6 6 12 12"/>',
    "lock" => '<rect width="18" height="11" x="3" y="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/>',
    "plus" => '<path d="M5 12h14"/><path d="M12 5v14"/>',
    "film" => '<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M7 3v18"/><path d="M3 7.5h4"/><path d="M3 12h18"/><path d="M3 16.5h4"/><path d="M17 3v18"/><path d="M17 7.5h4"/><path d="M17 16.5h4"/>',
    "activity" => '<path d="M22 12h-2.48a2 2 0 0 0-1.93 1.46l-2.35 8.36a.25.25 0 0 1-.48 0L9.24 2.18a.25.25 0 0 0-.48 0l-2.35 8.36A2 2 0 0 1 4.49 12H2"/>',
    "bot" => '<path d="M12 8V4H8"/><rect width="16" height="12" x="4" y="8" rx="2"/><path d="M2 14h2"/><path d="M20 14h2"/><path d="M15 13v2"/><path d="M9 13v2"/>',
    "plug" => '<path d="M12 22v-5"/><path d="M9 8V2"/><path d="M15 8V2"/><path d="M18 8v5a4 4 0 0 1-4 4h-4a4 4 0 0 1-4-4V8Z"/>',
    "scroll-text" => '<path d="M15 12h-5"/><path d="M15 8h-5"/><path d="M19 17V5a2 2 0 0 0-2-2H4"/><path d="M8 21h12a2 2 0 0 0 2-2v-1a1 1 0 0 0-1-1H11a1 1 0 0 0-1 1v1a2 2 0 1 1-4 0V5a2 2 0 1 0-4 0v2a1 1 0 0 0 1 1h3"/>',
    "database-backup" => '<ellipse cx="12" cy="5" rx="9" ry="3"/><path d="M3 12a9 3 0 0 0 5 2.69"/><path d="M21 9.3V5"/><path d="M3 5v14a9 3 0 0 0 6.47 2.88"/><path d="M12 12v4h4"/><path d="M13 20a5 5 0 0 0 9-3 4.5 4.5 0 0 0-4.5-4.5c-1.33 0-2.54.54-3.41 1.41L12 16"/>'
  }.freeze

  FEATURES = [
    { icon: "lock", title: "Named HTTPS", text: "Caddy serves each environment at a `.localhost` name with a local certificate, on a port that needs no sudo.", href: "/docs/faq/" },
    { icon: "plug", title: "Free ports", text: "Each HTTP service gets its own `$PORT`. Databases and other tools can ask for named ports. Running environments never share a port.", href: "/docs/configuration/#ports" },
    { icon: "activity", title: "Readiness checks", text: "`parcela up` waits until every healthcheck returns 2xx before it routes the URLs.", href: "/docs/configuration/#http-services" },
    { icon: "scroll-text", title: "Logs per process", text: "`parcela logs -f` follows every service, worker, and hook, with the process name on each line.", href: "/docs/cli/#logs" },
    { icon: "database-backup", title: "Data snapshots", text: "Snapshot the data of a stopped environment, then restore it into a new worktree to reproduce a test case.", href: "/docs/worktrees/#reproduce-a-test-dataset" },
    { icon: "bot", title: "Agent skill", text: "`parcela --skill` prints the instructions a coding agent needs to run Parcela.", href: "/docs/worktrees/#set-up-a-coding-agent" }
  ].freeze

  QUESTIONS = [
    [ "Does it work with my project?", "Parcela runs the commands you put in parcela.yaml. Use it with native web servers, workers, and setup scripts. It supports macOS and Linux, with jj workspaces or Git worktrees." ],
    [ "How separate are the environments?", "Each checkout gets its own processes, ports, and URLs. Your project configuration decides how databases, queues, and uploads are separated. Services run as native processes on your machine." ],
    [ "How does local HTTPS work?", "Caddy handles routing and TLS. The default address uses .localhost on port 1443. Your first terminal up offers to trust the local CA behind the system password prompt. Scripts and agents never change system trust." ],
    [ "What happens when I stop an environment?", "parcela down stops the environment's processes and removes its routes. Source and data are retained. LAN and Tailscale sharing stop with that run." ]
  ].freeze

  WORKTREES = [ [ "receipts", "Claude Code" ], [ "settle-up", "Codex" ], [ "group-stats", "Cursor" ] ].freeze

  # The Get started prompt that installs Parcela with a coding agent, copied by the hero's link.
  def agent_prompt
    guide = Rails.configuration.repository.join("docs/site/getting-started.md").read
    guide[/## Install it with your agent\n.*?```text\n(.*?)\n```/m, 1] or raise "getting-started.md has no agent prompt"
  end

  def brand(name)
    Docs::Markdown::BRANDS.find { it[:name] == name } or raise ArgumentError, "No brand #{name}"
  end

  # The agents in the hero, in their own colors.
  def home_agents
    AGENT_NAMES.map { |name| { name:, path: brand(name)[:path], color: AGENT_COLORS[name], gradient: AGENT_GRADIENTS[name] } }
  end

  def lucide(name, **attributes)
    tag.svg(LUCIDE.fetch(name).html_safe, viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", "stroke-width": attributes.delete(:stroke_width) || 2,
      "stroke-linecap": "round", "stroke-linejoin": "round", aria: { hidden: true }, focusable: "false", **attributes)
  end

  def brand_mark(path, **attributes)
    tag.svg(tag.path(d: path, fill: "currentColor"), viewBox: "0 0 24 24", aria: { hidden: true }, focusable: "false", **attributes)
  end

  # Copy whose backticked parts are inline code.
  def code_text(text)
    safe_join(text.split("`").each_with_index.map { |part, index| index.odd? ? tag.code(part, class: "inline-code") : part })
  end

  # A section title that links to itself, so any section can be shared as /#id.
  def section_heading(id, &block)
    tag.h2(id:, class: "section-heading") { link_to(capture(&block), "##{id}", class: "heading-link") }
  end

  # A hand-drawn underline that draws itself once the text scrolls into view.
  def pen_mark(text)
    tag.span(class: "pen-mark", data: { controller: "home-entrance", home_entrance_threshold_value: 0.5, home_entrance_margin_value: "0px 0px -15% 0px" }) do
      safe_join([ text, tag.svg(tag.path(pathLength: 1, d: "M3 13 C80 9 200 5 330 7 C410 8 465 10 497 15"), viewBox: "0 0 500 20", preserveAspectRatio: "none", aria: { hidden: true }, focusable: "false") ])
    end
  end

  def home_button(text, url, variant: :primary, start: false, arrow: true)
    classes = [ "home-button", ("home-start" if start), ("home-secondary" if variant == :secondary) ].compact.join(" ")
    lc_button_link_to(url, variant: start ? :primary : variant, size: start ? :lg : :md, class: classes) do
      safe_join([ (start_glint if start), text, (lucide("arrow-right", class: "home-arrow") if arrow) ].compact)
    end
  end

  # Two lights that sweep the start button's border, as superx.so's button glow.
  def start_glint
    tag.span(class: "start-glint", aria: { hidden: true }) { safe_join([ tag.i(class: "across"), tag.i(class: "down") ]) }
  end

  # The install command with a copy button, as a pill.
  def install_command(**attributes)
    tag.div(class: [ "install-command", attributes.delete(:class) ].compact.join(" "), **attributes) do
      safe_join([
        tag.code(safe_join(INSTALL_PARTS.map { |color, text| tag.span(text, style: "color: #{color}") }), class: "install-text"),
        lc_copy_button(value: INSTALL, icon_only: true, variant: :ghost, size: :sm, label: "Copy install command", class: "install-copy")
      ])
    end
  end

  # A laptop or phone around a page capture, in their real shapes: a MacBook's 16:10 and an
  # iPhone's 19.5:9. The address bar shows the secure local name the page is served at.
  def device_frame(image:, url:, kind: :laptop, alt: "", **attributes)
    host = url.delete_prefix("https://")
    tag.div(class: [ "device-frame", kind, attributes.delete(:class) ].compact.join(" "), **attributes) do
      safe_join([
        tag.div(class: "device-body") do
          tag.div(class: "device-glass") do
            safe_join([
              tag.div(class: "device-bar", aria: { label: url }) { tag.span(class: "device-field") { safe_join([ lucide("lock"), tag.span(host, class: "device-host") ]) } },
              tag.div(class: "device-page") { image_tag(image, alt:, decoding: "async", loading: "lazy") }
            ])
          end
        end,
        (tag.div(class: "device-base") if kind == :laptop)
      ].compact)
    end
  end
end
