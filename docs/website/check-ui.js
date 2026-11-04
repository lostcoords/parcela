async (page) => {
  const origin = /^https:\/\/[a-z0-9-]+\.localhost(?::\d+)?(?=\/)/i.exec(page.url())?.[0];
  if (!origin) {
    throw new Error('Open the named Parcela HTTPS URL before running this check; raw backend ports do not verify the preview.');
  }
  const check = (condition, message) => { if (!condition) throw new Error(message); };
  const click = (locator) => locator.evaluate((element) => element.click());
  const key = async (locator, value) => {
    await locator.dispatchEvent('keydown', { key: value, bubbles: true, cancelable: true });
    await page.evaluate(() => {
      if (!document.hasFocus()) document.activeElement?.dispatchEvent(new FocusEvent('focus'));
    });
  };
  const themeReady = () => page.waitForFunction(() => {
    const button = document.querySelector('.hero-actions .home-start');
    return button && getComputedStyle(button).backgroundColor === 'rgb(7, 102, 143)';
  }, null, { polling: 100, timeout: 5000 });
  const errors = [];
  const onError = (error) => errors.push(error.message);
  page.on('pageerror', onError);
  const originalViewport = page.viewportSize();
  let stage = 'homepage';
  try {
    await page.emulateMedia({ reducedMotion: 'reduce', colorScheme: 'light' });
    await page.setViewportSize({ width: 1440, height: 1000 });
    await page.goto(`${origin}/`);
    await page.evaluate(() => { scrollTo({ top: 0, behavior: 'instant' }); window.dispatchEvent(new Event('scroll')); });
    await page.locator('.video-placeholder').waitFor();
    if (await page.getByRole('switch', { name: 'Switch to light theme' }).isVisible()) {
      await click(page.getByRole('switch', { name: 'Switch to light theme' }));
    }
    await page.waitForFunction(() => !document.documentElement.classList.contains('dark'), null, { polling: 100, timeout: 5000 });
    await themeReady();
    const hero = page.getByRole('navigation', { name: 'Get Parcela' });
    check(await hero.locator('[data-slot="button"]').count() === 2, 'Hero actions use shadcn buttons');
    const primary = hero.getByRole('link', { name: 'Get started' });
    const light = await primary.evaluate((el) => {
      const style = getComputedStyle(el);
      return { background: style.backgroundColor, image: style.backgroundImage, shadow: style.boxShadow, height: el.offsetHeight, radius: style.borderRadius };
    });
    check(light.height >= 44 && parseFloat(light.radius) >= light.height / 2, 'Primary action has a pill shape and a touch target');
    check(light.image === 'none' && !light.shadow.includes('inset') && light.shadow !== 'none', 'Primary action has a solid surface and an outer glow');
    check(await page.getByRole('heading', { level: 1 }).count() === 1, 'Homepage has one clear heading');
    const anchors = await page.locator('.VPHome h2.section-heading').evaluateAll((headings) => headings.map((heading) => heading.querySelector('a.heading-link')?.getAttribute('href') === `#${heading.id}`));
    check(anchors.length === 7 && anchors.every(Boolean), 'Every section title links to its own anchor');
    const glint = await primary.evaluate((element) => getComputedStyle(element, '::before').animationName);
    check(glint === 'none', 'Primary action has no automatic animation');
    check(await page.locator('.heading .sr').textContent() === 'coding agents' && await page.locator('.rotator .agent.on').count() === 1, 'Rotating headline keeps a stable accessible label');
    check(await page.locator('.reveal').count() === 0, 'Reduced motion leaves homepage blocks unhidden');
    check(await page.locator('.share .device').evaluateAll((devices) => devices.every((device) => getComputedStyle(device).opacity === '1')), 'Reduced motion shows every sharing device');
    check(await page.locator('.share .arrows .stroke').evaluateAll((arrows) => arrows.length === 2 && arrows.every((arrow) => getComputedStyle(arrow).strokeDashoffset === '0px')), 'Reduced motion draws both sharing arrows');
    const resting = await page.locator('.dashboard video').evaluate((video) => ({ paused: video.paused, controls: video.controls, poster: !!video.poster }));
    check(resting.paused && resting.controls && resting.poster, 'Reduced motion leaves the dashboard recording on its poster, with controls');
    await page.waitForFunction(() => document.querySelector('.VPHome').getAnimations({ subtree: true }).every(animation => animation.playState !== 'running'), null, { polling: 100, timeout: 5000 });
    check(await page.locator('.hero-intro .availability, .hero-intro .agent-row').count() === 0, 'Hero omits the extra status and compatibility rows');
    const release = page.locator('.release-pill');
    const releaseTag = /v\d+\.\d+\.\d+/.exec(await release.innerText())?.[0];
    check(releaseTag && await release.getAttribute('href') === `https://github.com/lostcoords/parcela/releases/tag/${releaseTag}`, 'Hero links to the announced release');
    const borders = await page.locator('.home-section, .hero-actions a').evaluateAll((elements) =>
      elements.every((element) => getComputedStyle(element).borderTopWidth === '0px'));
    check(borders, 'Buttons and open sections have no decorative borders');
    const placeholder = page.locator('.video-placeholder');
    check(await placeholder.innerText() === 'Video demo coming soon', 'Video space has a clear placeholder label');
    const videoSpace = await placeholder.boundingBox();
    check(Math.abs(videoSpace.width / videoSpace.height - 16 / 9) < 0.01, 'Video space reserves a 16:9 frame');
    check(await placeholder.locator('button, a, video').count() === 0, 'Placeholder has no inactive playback controls');
    check(await placeholder.evaluate(element => element.getAnimations({ subtree: true }).length) === 0, 'Video placeholder stays static');
    const pill = await release.evaluate((element) => {
      const style = getComputedStyle(element);
      return { radius: parseFloat(style.borderRadius), height: element.offsetHeight, shadow: style.boxShadow, blur: style.backdropFilter };
    });
    check(pill.radius >= pill.height / 2 && pill.shadow !== 'none' && pill.blur.includes('blur'), 'Release announcement is a glass pill');
    check(await page.locator('.share figure').count() === 3, 'Sharing shows this machine, a phone and a tailnet device');
    stage = 'same app on every device';
    const screens = page.locator('.share .screen img, .bento .pair img');
    check(await screens.count() === 5, 'Every device frame shows a screen');
    for (const screen of await screens.all()) {
      await screen.scrollIntoViewIfNeeded();
      check(await screen.evaluate((image) => image.decode().then(() => image.naturalWidth > 0)), 'Device screens load their capture');
    }
    await page.evaluate(() => scrollTo(0, 0));

    stage = 'homepage questions';
    const question = page.locator('summary').filter({ hasText: 'How separate are the environments?' });
    await click(question);
    check(await page.getByText('Each checkout gets its own processes, ports, and URLs.', { exact: false }).isVisible(), 'FAQ opens its answer');
    await click(question);

    stage = 'header';
    await page.waitForFunction(() => document.querySelector('.VPNavBar')?.classList.contains('top'), null, { polling: 100, timeout: 5000 });
    const texture = await page.locator('.VPHome').evaluate((element) => ({
      dots: getComputedStyle(element, '::before').backgroundImage,
    }));
    check(texture.dots === 'none', 'Homepage keeps a plain background');
    const headerTop = await page.locator('.VPNavBar .wrapper').evaluate((element) => ({
      blur: getComputedStyle(element).backdropFilter,
      radius: getComputedStyle(element).borderRadius,
    }));
    const homeHeaderWidth = await page.locator('.VPNavBar .wrapper').evaluate((element) => element.getBoundingClientRect().width);
    check(homeHeaderWidth === 1440, 'Homepage header spans the viewport before scrolling');
    const serverHeader = await page.locator('.VPNavBar').evaluate((navbar) => {
      const initial = navbar.cloneNode(true);
      initial.className = 'VPNavBar';
      navbar.parentElement.append(initial);
      const wrapper = initial.querySelector('.wrapper');
      const result = { width: wrapper.getBoundingClientRect().width, radius: getComputedStyle(wrapper).borderRadius };
      initial.remove();
      return result;
    });
    check(serverHeader.width === 1440 && serverHeader.radius === '0px', 'Navbar starts full width before VitePress initializes scroll classes');
    check(headerTop.blur.includes('blur(0px)') && headerTop.radius === '0px', 'Header is flat at the top');
    const searchBox = await page.locator('.DocSearch-Button').boundingBox();
    const sectionsBox = await page.locator('.nav-sections').boundingBox();
    check(searchBox.width <= 160 && searchBox.width >= 40, 'Desktop search stays compact');
    check(sectionsBox.x + sectionsBox.width <= searchBox.x, 'Section links precede search visually');
    check(await page.locator('.nav-sections').evaluate((menu) => !!(menu.compareDocumentPosition(document.querySelector('.VPNavBarSearch')) & Node.DOCUMENT_POSITION_FOLLOWING)), 'Section links precede search in keyboard order');
    await page.evaluate(() => { scrollTo({ top: 300, behavior: 'instant' }); window.dispatchEvent(new Event('scroll')); });
    await page.waitForFunction(() => !document.querySelector('.VPNavBar')?.classList.contains('top'), null, { polling: 100, timeout: 5000 });
    const floating = await page.locator('.VPNavBar .wrapper').evaluate((element) => ({
      blur: getComputedStyle(element).backdropFilter,
      radius: getComputedStyle(element).borderRadius,
      top: element.getBoundingClientRect().top,
      height: element.getBoundingClientRect().height,
    }));
    check(floating.blur.includes('blur') && floating.radius === '32px' && floating.top === 8 && floating.height >= 64, 'Floating glass header has generous vertical space');
    await page.evaluate(() => { scrollTo({ top: 0, behavior: 'instant' }); window.dispatchEvent(new Event('scroll')); });
    await page.waitForFunction(() => document.querySelector('.VPNavBar')?.classList.contains('top'), null, { polling: 100, timeout: 5000 });

    const alignment = await page.locator('.demo').evaluate(element => {
      const frame = element.querySelector('.video-placeholder').getBoundingClientRect();
      const sections = [...document.querySelectorAll('.home-section')];
      return sections.length >= 6 && sections.every((section) => {
        const box = section.getBoundingClientRect();
        const style = getComputedStyle(section);
        const left = box.left + parseFloat(style.paddingLeft);
        const right = box.right - parseFloat(style.paddingRight);
        return Math.abs(frame.left - left) <= 1 && Math.abs(frame.right - right) <= 1;
      });
    });
    check(alignment, 'Video space and every section share the page width and gutters');

    stage = 'dark theme';
    await click(page.getByRole('switch', { name: 'Switch to dark theme' }));
    await page.waitForFunction(() => document.documentElement.classList.contains('dark'), null, { polling: 100, timeout: 5000 });
    await themeReady();
    const dark = await primary.evaluate((el) => getComputedStyle(el).backgroundColor);
    check(dark === light.background, 'Accent keeps its color in both themes');
    const shared = await page.evaluate(() => {
      const probe = document.createElement('div');
      probe.style.backgroundColor = 'var(--vp-c-bg)';
      document.body.append(probe);
      const matches = getComputedStyle(probe).backgroundColor === getComputedStyle(document.body).backgroundColor;
      probe.remove();
      return matches;
    });
    check(shared, 'VitePress and the page use the same dark surface');
    const neutrals = await page.evaluate(() => {
      const probe = document.createElement('span');
      document.body.append(probe);
      const normalized = (value) => { probe.style.color = value; return getComputedStyle(probe).color; };
      const pairs = [
        ['--background', 'oklch(20.5% 0 0)'],
        ['--secondary', 'oklch(24.2% 0 0)'],
        ['--muted', 'oklch(28.4% 0 0)'],
        ['--foreground', 'oklch(91.5% 0 0)'],
        ['--primary', 'oklch(91.5% 0 0)'],
      ];
      const matches = pairs.every(([token, expected]) => normalized(`var(${token})`) === normalized(expected));
      probe.remove();
      return matches;
    });
    check(neutrals, 'Dark surfaces, text and primary buttons match Lostcoords UI neutrals');

    stage = 'documentation and search';
    await click(primary);
    await page.waitForURL('**/docs/getting-started/');
    await page.locator('.vp-doc h1').filter({ hasText: 'Get started' }).waitFor();
    check(await page.locator('h1').count() === 1, 'Get started opens a documentation page');
    check(await page.locator('.vp-doc button.copy').count() > 0, 'Documentation keeps copy controls');
    await page.waitForFunction(() => {
      const block = document.querySelector('.vp-doc div[class*="language-"]');
      return block && getComputedStyle(block).borderTopWidth === '0px' && getComputedStyle(block).boxShadow === 'none';
    }, null, { polling: 100, timeout: 5000 });
    const codeAppearance = await page.locator('.vp-doc div[class*="language-"]').first().evaluate((element) => {
      const style = getComputedStyle(element);
      return { border: style.borderTopWidth, shadow: style.boxShadow };
    });
    check(codeAppearance.border === '0px' && codeAppearance.shadow === 'none', `Documentation code has no border or shadow: ${JSON.stringify(codeAppearance)}`);
    const docsHeader = await page.locator('.VPNavBar .wrapper').evaluate((element) => ({ width: element.getBoundingClientRect().width, radius: getComputedStyle(element).borderRadius, blur: getComputedStyle(element).backdropFilter }));
    await page.evaluate(() => { scrollTo({ top: 300, behavior: 'instant' }); window.dispatchEvent(new Event('scroll')); });
    await page.waitForFunction(() => !document.querySelector('.VPNavBar')?.classList.contains('top'), null, { polling: 100, timeout: 5000 });
    const docsScrolled = await page.locator('.VPNavBar .wrapper').evaluate((element) => ({ width: element.getBoundingClientRect().width, radius: getComputedStyle(element).borderRadius, blur: getComputedStyle(element).backdropFilter }));
    check(docsHeader.width === homeHeaderWidth && docsScrolled.width === homeHeaderWidth && docsScrolled.radius === '0px' && docsScrolled.blur === 'none', 'Docs header stays full width without the homepage scroll effect');
    const tableAppearance = await page.locator('.vp-doc table').first().evaluate((element) => [element, ...element.querySelectorAll('tr, th, td')].every((part) => {
      const style = getComputedStyle(part);
      return style.borderTopWidth === '0px' && style.backgroundColor === 'rgba(0, 0, 0, 0)';
    }));
    check(tableAppearance, 'Documentation tables have no frames or stripes');
    await click(page.getByRole('button', { name: 'Search', exact: true }));
    await page.locator('#localsearch-input').fill('snapshot');
    await page.getByRole('option', { name: /snapshot/i }).first().waitFor({ timeout: 5000 });
    await key(page.locator('#localsearch-input'), 'Escape');

    stage = 'tablet layout';
    for (const width of [768, 1024]) {
      await page.setViewportSize({ width, height: 1000 });
      await page.goto(`${origin}/`);
      check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `Homepage fits ${width}px`);
      const frame = await placeholder.boundingBox();
      check(Math.abs(frame.width / frame.height - 16 / 9) < 0.01, `Video space preserves its aspect ratio at ${width}px`);
    }
    stage = 'mobile navigation';
    for (const width of [390, 320]) {
      await page.setViewportSize({ width, height: 844 });
      await page.goto(`${origin}/`);
      check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `Homepage fits ${width}px`);
      const frame = await placeholder.boundingBox();
      check(Math.abs(frame.width / frame.height - 16 / 9) < 0.01, `Video space preserves its aspect ratio at ${width}px`);
      check(await page.evaluate(() => document.querySelector('.release-pill').getBoundingClientRect().top >= document.querySelector('.VPNavBar').getBoundingClientRect().bottom), `Hero clears the header at ${width}px`);
      check((await page.locator('.share .phone figcaption code').allTextContents()).includes('parcela up --lan'), 'Phone caption identifies LAN sharing');
      check((await page.locator('.share .tailnet figcaption code').allTextContents()).includes('parcela up --tailscale'), 'Remote computer caption identifies Tailscale sharing');
      await page.evaluate(() => { scrollTo({ top: 300, behavior: 'instant' }); window.dispatchEvent(new Event('scroll')); });
      await page.waitForFunction(() => !document.querySelector('.VPNavBar')?.classList.contains('top'), null, { polling: 100, timeout: 5000 });
      check(await page.locator('.VPNavBar .wrapper').evaluate((element) => element.getBoundingClientRect().top) === 8, 'Mobile header floats while scrolling');
      const menuIconCentered = await page.locator('.VPNavBarHamburger').evaluate((button) => {
        const icon = button.querySelector('.container').getBoundingClientRect();
        const target = button.getBoundingClientRect();
        return icon.height <= 20 && Math.abs(icon.top + icon.height / 2 - target.top - target.height / 2) <= 1;
      });
      check(menuIconCentered, 'Mobile menu icon stays centered inside the taller navbar');
      await click(page.getByRole('button', { name: 'mobile navigation' }));
      await click(page.locator('#VPNavScreen').getByRole('link', { name: 'Get started', exact: true }));
      await page.waitForURL('**/docs/getting-started/');
      check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `Documentation fits ${width}px`);
      check(await page.getByRole('heading', { level: 1 }).isVisible(), 'Mobile navigation reaches the docs');
    }
    stage = 'header on scroll';
    await page.setViewportSize({ width: 1440, height: 1000 });
    await page.emulateMedia({ reducedMotion: 'no-preference' });
    await page.goto(`${origin}/`);
    await page.evaluate(() => document.fonts.ready);
    await page.evaluate(() => { scrollTo({ top: 0, behavior: 'instant' }); window.dispatchEvent(new Event('scroll')); });
    await page.waitForFunction(() => document.querySelector('.VPNavBar')?.classList.contains('top'), null, { polling: 100, timeout: 5000 });
    await page.evaluate(() => {
      for (const animation of document.getAnimations()) {
        if (animation instanceof CSSTransition) animation.finish();
      }
    });
    const fullWidth = await page.locator('.VPNavBar .wrapper').evaluate((element) => element.getBoundingClientRect().width);
    await page.evaluate(() => { scrollTo({ top: 300, behavior: 'instant' }); window.dispatchEvent(new Event('scroll')); });
    await page.waitForFunction(() => !document.querySelector('.VPNavBar')?.classList.contains('top'), null, { polling: 100, timeout: 5000 });
    const morph = await page.locator('.VPNavBar .wrapper').evaluate((wrapper) => {
      const animation = wrapper.getAnimations().find((item) => item instanceof CSSTransition && item.transitionProperty === 'max-width');
      if (!animation) return null;
      animation.pause();
      animation.currentTime = 0;
      const start = wrapper.getBoundingClientRect().width;
      animation.currentTime = 200;
      const middle = wrapper.getBoundingClientRect().width;
      animation.finish();
      const end = wrapper.getBoundingClientRect().width;
      for (const item of document.getAnimations()) {
        if (item instanceof CSSTransition) item.finish();
      }
      return { start, middle, end };
    });
    check(morph && fullWidth === morph.start && morph.start > morph.middle && morph.middle > morph.end, 'Seamless header animates into a compact glass navbar');

    stage = 'sharing motion on a short screen';
    await page.setViewportSize({ width: 844, height: 390 });
    await page.goto(`${origin}/`);
    await page.locator('.share').evaluate((element) => element.scrollIntoView({ block: 'center' }));
    await page.waitForFunction(() => document.querySelector('.share')?.classList.contains('playing'), null, { polling: 100, timeout: 5000 });

    stage = 'restored homepage animations';
    await page.setViewportSize({ width: 1440, height: 1000 });
    await page.goto(`${origin}/`);
    await page.evaluate(() => document.fonts.ready);
    await page.waitForFunction(() => document.querySelector('.share')?.classList.contains('armed'), null, { polling: 100, timeout: 5000 });
    const firstAgent = await page.locator('.rotator .agent.on').innerText();
    await page.locator('.dashboard video').evaluate((video) => video.scrollIntoView({ block: 'center' }));
    await page.waitForFunction(() => { const video = document.querySelector('.dashboard video'); return !video.paused && video.currentTime > 0.5 && !video.controls; }, null, { polling: 100, timeout: 8000 });
    await page.waitForFunction((name) => document.querySelector('.rotator .agent.on')?.textContent !== name, firstAgent, { polling: 100, timeout: 8000 });
    stage = 'sharing motion';
    await page.locator('.share').evaluate(element => element.scrollIntoView({ block: 'center' }));
    await page.waitForFunction(() => document.querySelector('.share')?.classList.contains('playing'), null, { polling: 100, timeout: 5000 });
    await page.waitForFunction(() => [...document.querySelectorAll('.share .device')].every((device) => getComputedStyle(device).opacity === '1')
      && [...document.querySelectorAll('.share .arrows path')].every((path) => getComputedStyle(path).strokeDashoffset === '0px'), null, { polling: 100, timeout: 5000 });
    check(true, 'Sharing arrows draw from this machine to each device');
    await page.locator('.cta').evaluate(element => element.scrollIntoView({ block: 'center' }));
    check(await page.locator('.mascot').evaluate(element => element.getAnimations().some(animation => animation.effect.getComputedTiming().iterations === Infinity)), 'CTA mascot sways');
    await page.waitForFunction(() => document.querySelector('.mascot-reveal')?.classList.contains('revealed'), null, { polling: 100, timeout: 5000 });
    check(true, 'CTA mascot rises into view');
    const blocks = await page.locator('.reveal').count();
    for (let top = 0; top < await page.evaluate(() => document.documentElement.scrollHeight); top += 500) {
      await page.evaluate((y) => scrollTo({ top: y, behavior: 'instant' }), top);
      await page.waitForTimeout(60);
    }
    await page.waitForFunction(() => document.querySelectorAll('.reveal:not(.revealed)').length === 0, null, { polling: 100, timeout: 5000 });
    check(blocks >= 30, `Every homepage block reveals on scroll (${blocks})`);

    stage = 'live reduced-motion preference';
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.screenshot();
    check(await page.evaluate(() => document.getAnimations().filter(animation => animation.playState === 'running').length) === 0, 'Changing to reduced motion stops running animations');
    const restingAgent = await page.locator('.rotator .agent.on').innerText();
    await page.waitForTimeout(2500);
    check(await page.locator('.rotator .agent.on').innerText() === restingAgent, 'Reduced motion stops headline rotation');
    check(await placeholder.isVisible(), 'Video placeholder remains visible with reduced motion');
    check(errors.length === 0, `Browser errors: ${errors.join(', ')}`);
    return { passed: ['static 16:9 video placeholder', 'standard page width and responsive gutters', 'same app on three devices', 'plain background and scroll header', 'FAQ answers and reduced motion', 'rotating headline', 'sharing arrows', 'scroll reveals on every block', 'live reduced-motion preference', 'borderless documentation code and tables', 'light and dark theme', 'local search', 'mobile navigation', '320px, 390px, 768px and 1024px layouts'], clipboard: 'untouched' };
  } catch (error) {
    const demoState = await page.locator('.demo').evaluate(element => ({ classes: element.className, text: element.innerText.slice(-400) })).catch(() => null);
    throw new Error(`${stage}: ${error.message}; demo=${JSON.stringify(demoState)}`);
  } finally {
    page.off('pageerror', onError);
    await page.emulateMedia({ reducedMotion: null, colorScheme: null });
    if (originalViewport) await page.setViewportSize(originalViewport);
    await page.goto(`${origin}/`);
  }
}
