// The Carbon web components the page uses, bundled into src/vendor/carbon.js by
// scripts/vendor-carbon.mjs. Importing a component registers its `cds-*` element. A component the
// page starts using must be added here, and the vendor files rebuilt (`npm run vendor`).
import '@carbon/web-components/es/components/ui-shell/header.js';
import '@carbon/web-components/es/components/ui-shell/header-name.js';
import '@carbon/web-components/es/components/ui-shell/side-nav.js';
import '@carbon/web-components/es/components/ui-shell/side-nav-items.js';
import '@carbon/web-components/es/components/ui-shell/side-nav-link.js';
import '@carbon/web-components/es/components/breadcrumb/breadcrumb.js';
import '@carbon/web-components/es/components/breadcrumb/breadcrumb-item.js';
import '@carbon/web-components/es/components/breadcrumb/breadcrumb-link.js';
import '@carbon/web-components/es/components/button/button.js';
import '@carbon/web-components/es/components/button/button-set.js';
import '@carbon/web-components/es/components/tile/tile.js';
import '@carbon/web-components/es/components/tag/tag.js';
import '@carbon/web-components/es/components/overflow-menu/overflow-menu.js';
import '@carbon/web-components/es/components/overflow-menu/overflow-menu-body.js';
import '@carbon/web-components/es/components/overflow-menu/overflow-menu-item.js';
import '@carbon/web-components/es/components/text-input/text-input.js';
import '@carbon/web-components/es/components/textarea/textarea.js';
import '@carbon/web-components/es/components/password-input/password-input.js';
import '@carbon/web-components/es/components/checkbox/checkbox.js';
import '@carbon/web-components/es/components/checkbox/checkbox-group.js';
import '@carbon/web-components/es/components/select/select.js';
import '@carbon/web-components/es/components/select/select-item.js';
import '@carbon/web-components/es/components/notification/inline-notification.js';
import '@carbon/web-components/es/components/modal/modal.js';
import '@carbon/web-components/es/components/modal/modal-header.js';
import '@carbon/web-components/es/components/modal/modal-heading.js';
import '@carbon/web-components/es/components/modal/modal-close-button.js';
import '@carbon/web-components/es/components/modal/modal-body.js';
import '@carbon/web-components/es/components/modal/modal-body-content.js';
import '@carbon/web-components/es/components/modal/modal-footer.js';
import '@carbon/web-components/es/components/modal/modal-footer-button.js';
import '@carbon/web-components/es/components/inline-loading/inline-loading.js';

// Carbon icons, as `carbonIcon(name)`: an <svg> element, 16 px unless another size is given.
import { toSVG } from '@carbon/icon-helpers';
import add16 from '@carbon/icons/es/add/16.js';
import edit16 from '@carbon/icons/es/edit/16.js';
import locked16 from '@carbon/icons/es/locked/16.js';
import overflowMenuVertical16 from '@carbon/icons/es/overflow-menu--vertical/16.js';
import play16 from '@carbon/icons/es/play/16.js';
import listBoxes16 from '@carbon/icons/es/list--boxes/16.js';
import settings16 from '@carbon/icons/es/settings/16.js';
import stop16 from '@carbon/icons/es/stop/16.js';
import trashCan16 from '@carbon/icons/es/trash-can/16.js';

const icons = {
  add: add16,
  edit: edit16,
  'list-boxes': listBoxes16,
  locked: locked16,
  'overflow-menu-vertical': overflowMenuVertical16,
  play: play16,
  settings: settings16,
  stop: stop16,
  'trash-can': trashCan16,
};

window.carbonIcon = (name, slot) => {
  const svg = toSVG(icons[name]);
  if (slot) svg.setAttribute('slot', slot);
  return svg;
};
