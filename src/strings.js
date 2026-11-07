// Interface text in Chinese and English, picked from the system language.

const zh = (navigator.language || '').toLowerCase().startsWith('zh');
const pick = (zhText, enText) => (zh ? zhText : enText);

export const lang = zh ? 'zh' : 'en';

export const t = {
  appName: 'TackNote',
  search: pick('搜索便签', 'Search notes'),
  newNote: pick('新便签', 'New note'),
  emptyTitle: pick('还没有便签', 'No notes yet'),
  emptyBody: pick('点右下角的按钮,写下第一张。', 'Tap the button to write your first one.'),
  noResults: pick('没有找到匹配的便签', 'No matching notes'),
  placeholder: pick('写点什么……', 'Write something…'),
  back: pick('返回', 'Back'),
  pin: pick('置顶', 'Pin'),
  unpin: pick('取消置顶', 'Unpin'),
  delete: pick('删除', 'Delete'),
  addCheckbox: pick('添加待办', 'Add checkbox'),
  color: (n) => pick(`颜色 ${n}`, `Colour ${n}`),
  conflictCopy: pick('冲突副本', 'Conflict copy'),
  settings: pick('同步设置', 'Sync settings'),
  webdavUrl: pick('WebDAV 地址', 'WebDAV URL'),
  username: pick('用户名', 'Username'),
  password: pick('密码', 'Password'),
  saveAndSync: pick('保存并同步', 'Save and sync'),
  saving: pick('正在保存……', 'Saving…'),
  insecureUrl: pick(
    '这个地址没有加密,密码和便签可能被别人看到。建议改用 https。',
    'This address is not encrypted, so others could read your password and notes. Use https if you can.',
  ),
  settingsHint: pick(
    '便签保存在你自己的 WebDAV 空间里。每台设备填同一个账号,就会自动同步。',
    'Notes are stored in your own WebDAV space. Use the same account on each device to keep them in sync.',
  ),
  passwordUnavailable: pick(
    '读不到保存的密码,请重新输入后保存。',
    'The saved password could not be read. Enter it again and save.',
  ),
  keychainFailed: pick(
    '密码没能存进系统钥匙串,所以这次什么都没有保存。',
    'The password could not be stored in the system keychain, so nothing was saved.',
  ),
  saveFailed: pick('没能保存到这台设备,请稍后再试。', 'Could not save on this device. Please try again.'),
  openFailed: pick(
    '打不开这台设备上保存的便签。为了不覆盖它们,应用暂时停用了。',
    'The notes saved on this device could not be opened. The app has stopped so they are not overwritten.',
  ),
  lastSynced: (time) => pick(`上次同步:${time}`, `Last synced: ${time}`),
  conflicts: (count) => pick(
    `有 ${count} 张便签在两处同时被改,已保留冲突副本`,
    `${count} note(s) changed in two places. A conflict copy was kept.`,
  ),
  status(status, error) {
    switch (status) {
      case 'notConfigured': return pick('未设置同步', 'Sync not set up');
      case 'syncing': return pick('正在同步', 'Syncing');
      case 'ok': return pick('已同步', 'Synced');
      default: return syncError(error);
    }
  },
};

function syncError(error) {
  switch (error) {
    case 'auth': return pick('账号或密码不对', 'Wrong username or password');
    case 'credentialsUnavailable': return pick(
      '读不到保存的密码,请到同步设置里重新输入',
      'Could not read the saved password. Enter it again in sync settings.',
    );
    case 'badUrl': return pick('WebDAV 地址格式不对,请检查一下', 'The WebDAV address is not valid. Please check it.');
    case 'network': return pick('连不上服务器', 'Could not reach the server');
    case 'corrupt': return pick(
      '服务器上的文件已损坏,没有覆盖它',
      'The file on the server is damaged and was left untouched',
    );
    case 'unsupportedVersion': return pick(
      '服务器上的数据来自更新的版本,请先升级应用',
      'The server data is from a newer version. Update the app first.',
    );
    case 'localStorage': return pick(
      '同步到的便签没能保存到这台设备',
      'The synced notes could not be saved on this device',
    );
    default: return pick('同步失败,稍后会重试', 'Sync failed. Will retry soon.');
  }
}
