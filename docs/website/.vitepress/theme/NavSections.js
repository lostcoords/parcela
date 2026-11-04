import { h } from 'vue';
import { useData } from 'vitepress';
import VPNavBarMenuLink from 'vitepress/dist/client/theme-default/components/VPNavBarMenuLink.vue';
import VPNavBarMenuGroup from 'vitepress/dist/client/theme-default/components/VPNavBarMenuGroup.vue';

export default function NavSections() {
  const { theme } = useData();
  return h('nav', { class: 'nav-sections', 'aria-label': 'Main Navigation' },
    theme.value.nav.map((item) => {
      if ('component' in item) return h(item.component, item.props);
      return h('link' in item ? VPNavBarMenuLink : VPNavBarMenuGroup, { item });
    }),
  );
}
