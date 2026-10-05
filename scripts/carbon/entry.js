// The Carbon web components the page uses, bundled into src/vendor/carbon.js by
// scripts/vendor-carbon.mjs. Importing a component registers its `cds-*` element. A component the
// page starts using must be added here, and the vendor files rebuilt (`npm run vendor`).
import '@carbon/web-components/es/components/ui-shell/header.js';
import '@carbon/web-components/es/components/ui-shell/header-name.js';
import '@carbon/web-components/es/components/ui-shell/header-global-action.js';
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
// The until lock's end: a date and a time of day.
import '@carbon/web-components/es/components/date-picker/date-picker.js';
import '@carbon/web-components/es/components/date-picker/date-picker-input.js';
import '@carbon/web-components/es/components/time-picker/time-picker.js';
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
import calendar16 from '@carbon/icons/es/calendar/16.js';
import calendar20 from '@carbon/icons/es/calendar/20.js';
import locked16 from '@carbon/icons/es/locked/16.js';
import overflowMenuVertical16 from '@carbon/icons/es/overflow-menu--vertical/16.js';
import play16 from '@carbon/icons/es/play/16.js';
import home20 from '@carbon/icons/es/home/20.js';
import listChecked16 from '@carbon/icons/es/list--checked/16.js';
import listChecked20 from '@carbon/icons/es/list--checked/20.js';
import settings20 from '@carbon/icons/es/settings/20.js';
import stop16 from '@carbon/icons/es/stop/16.js';
import time16 from '@carbon/icons/es/time/16.js';
import trashCan16 from '@carbon/icons/es/trash-can/16.js';
import unlocked16 from '@carbon/icons/es/unlocked/16.js';

const icons = {
  add: add16,
  calendar: calendar20,
  // The 16 px ones in front of a block tile's lines; the 20 px ones are the header's.
  'calendar-16': calendar16,
  home: home20,
  'list-checked': listChecked20,
  'list-checked-16': listChecked16,
  locked: locked16,
  'overflow-menu-vertical': overflowMenuVertical16,
  play: play16,
  settings: settings20,
  stop: stop16,
  time: time16,
  'trash-can': trashCan16,
  unlocked: unlocked16,
};

window.carbonIcon = (name, slot) => {
  const svg = toSVG(icons[name]);
  if (slot) svg.setAttribute('slot', slot);
  return svg;
};
