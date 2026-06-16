/**
 * Chinese display labels for journal area values returned by index databases.
 */

const CHINESE_AREA_LABELS: Record<string, string> = {
  'Accounting & Auditing': '会计与审计',
  'Agricultural Economics': '农业经济学',
  'Business History': '商业史',
  Economics: '经济学',
  'Education Management': '教育管理',
  'Engineering & Industrial Management': '工程与工业管理',
  'Entrepreneurship & Small Business': '创业与小企业',
  Finance: '金融学',
  'Health Management & Economics': '健康管理与经济学',
  'Human Resources & Employment': '人力资源与就业',
  'Information & Library Science': '信息与图书馆学',
  'Information Systems': '信息系统',
  'International Business & Affairs': '国际商务与事务',
  'Management & Organization': '管理与组织',
  Marketing: '市场营销',
  'Operations & Management Science': '运营与管理科学',
  'Psychology & Behavior': '心理学与行为',
  'Public Administration & Policy': '公共管理与政策',
  'Regional, Environmental & Resource Studies': '区域、环境与资源研究',
  'Risk & Safety Management': '风险与安全管理',
  'Social Sciences': '社会科学',
  'Statistics & Econometrics': '统计学与计量经济学',
  'Strategy & Innovation': '战略与创新',
  'Tourism, Hospitality & Transportation': '旅游、酒店与交通',
};

/**
 * Resolve the user-facing Chinese label for a journal area.
 *
 * @param area - Raw area value from the index database.
 * @returns Chinese display label when known; otherwise the original value.
 */
export function getAreaDisplayName(area: string): string {
  const normalizedArea = area.trim();
  return CHINESE_AREA_LABELS[normalizedArea] ?? area;
}
