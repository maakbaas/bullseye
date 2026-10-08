// The Carbon web components the page uses, bundled into src/vendor/carbon.js by
// scripts/vendor-carbon.mjs. Importing a component registers its `cds-*` element. A component the
// page starts using must be added here, and the vendor files rebuilt (`npm run vendor`).
import '@carbon/web-components/es/components/ui-shell/header.js';
import '@carbon/web-components/es/components/ui-shell/header-name.js';
// The pages, as tabs under the header.
import '@carbon/web-components/es/components/tabs/tabs.js';
import '@carbon/web-components/es/components/tabs/tab.js';
// An icon-only tab (narrow window) names itself in a tooltip.
import '@carbon/web-components/es/components/tooltip/tooltip.js';
import '@carbon/web-components/es/components/tooltip/tooltip-content.js';
import '@carbon/web-components/es/components/breadcrumb/breadcrumb.js';
import '@carbon/web-components/es/components/breadcrumb/breadcrumb-item.js';
import '@carbon/web-components/es/components/breadcrumb/breadcrumb-link.js';
import '@carbon/web-components/es/components/link/link.js';
import '@carbon/web-components/es/components/button/button.js';
import '@carbon/web-components/es/components/button/button-set.js';
// Edit, beside each block list in the block editor: an icon that names itself in a tooltip.
import '@carbon/web-components/es/components/icon-button/icon-button.js';
// The block list editor's sections.
import '@carbon/web-components/es/components/accordion/accordion.js';
import '@carbon/web-components/es/components/accordion/accordion-item.js';
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
// One choice of several: how the windows by title are dealt with (close or minimize).
import '@carbon/web-components/es/components/radio-button/radio-button.js';
import '@carbon/web-components/es/components/radio-button/radio-button-group.js';
import '@carbon/web-components/es/components/dropdown/dropdown.js';
import '@carbon/web-components/es/components/dropdown/dropdown-item.js';
import '@carbon/web-components/es/components/multi-select/multi-select.js';
import '@carbon/web-components/es/components/multi-select/multi-select-item.js';
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
// A session's settings on its tile: one row each, whose edit button opens what it can be.
import '@carbon/web-components/es/components/contained-list/contained-list.js';
import '@carbon/web-components/es/components/contained-list/contained-list-item.js';
// A row's edit button: a tab-tip popover, the button and the open panel one shape.
import '@carbon/web-components/es/components/popover/popover.js';
import '@carbon/web-components/es/components/popover/popover-content.js';

// Carbon icons, as `carbonIcon(name)`: an <svg> element, 16 px unless another size is given.
import { getAttributes, toSVG } from '@carbon/icon-helpers';
import add16 from '@carbon/icons/es/add/16.js';
import checkmark16 from '@carbon/icons/es/checkmark/16.js';
import close16 from '@carbon/icons/es/close/16.js';
import calendar16 from '@carbon/icons/es/calendar/16.js';
import edit16 from '@carbon/icons/es/edit/16.js';
import documentExport16 from '@carbon/icons/es/document--export/16.js';
import documentImport16 from '@carbon/icons/es/document--import/16.js';
import locked16 from '@carbon/icons/es/locked/16.js';
import overflowMenuVertical16 from '@carbon/icons/es/overflow-menu--vertical/16.js';
import play16 from '@carbon/icons/es/play/16.js';
import plug16 from '@carbon/icons/es/plug/16.js';
import renew16 from '@carbon/icons/es/renew/16.js';
import rule16 from '@carbon/icons/es/rule/16.js';
import listChecked16 from '@carbon/icons/es/list--checked/16.js';
import settings16 from '@carbon/icons/es/settings/16.js';
import stop16 from '@carbon/icons/es/stop/16.js';
import time16 from '@carbon/icons/es/time/16.js';
import pause16 from '@carbon/icons/es/pause/16.js';
import trashCan16 from '@carbon/icons/es/trash-can/16.js';
import unlocked16 from '@carbon/icons/es/unlocked/16.js';
import chevronDown16 from '@carbon/icons/es/chevron--down/16.js';

const icons = {
  add: add16,
  // In front of a block tile's lines and the tabs' labels.
  'calendar-16': calendar16,
  // Export and Import in Settings.
  // Edit, beside a block's block lists and schedule.
  edit: edit16,
  // Buttons: Save and the dialog's confirm, Cancel, and Scan again.
  checkmark: checkmark16,
  close: close16,
  renew: renew16,
  'document-export': documentExport16,
  'document-import': documentImport16,
  'list-checked-16': listChecked16,
  locked: locked16,
  'overflow-menu-vertical': overflowMenuVertical16,
  play: play16,
  // A flash drive condition's line on a block's tile; the Conditions tab.
  'plug-16': plug16,
  'rule-16': rule16,
  settings: settings16,
  stop: stop16,
  time: time16,
  // The Allowances tab, and an allowance's line on a block's tile.
  'pause-16': pause16,
  'trash-can': trashCan16,
  unlocked: unlocked16,
  // At the end of a session's row that can be changed: it opens a menu.
  'chevron-down': chevronDown16,
};

window.carbonIcon = (name, slot) => {
  // `toSVG` adds Carbon's default attributes (aria-hidden, preserveAspectRatio, …) but takes their
  // values from the icon's own, so they came out "undefined": given them already merged, it sets them.
  const icon = icons[name];
  const svg = toSVG({ ...icon, attrs: getAttributes(icon.attrs) });
  if (slot) svg.setAttribute('slot', slot);
  return svg;
};
