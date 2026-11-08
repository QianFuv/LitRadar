/** Presentation labels for backend-classified CFP kinds and milestones. */

import type { CfpDateStage, CfpKind } from '@/lib/api';

export const CFP_KIND_LABELS: Readonly<Record<CfpKind, string>> = {
  special_issue: '专题征稿',
  general: '常规征稿 / 年度选题',
  proposal: '特刊提案',
  conference_linked: '会议关联',
};

export const CFP_DATE_LABELS: Readonly<Record<CfpDateStage, string>> = {
  opens: '开始投稿',
  paper: '论文截止',
  abstract: '摘要截止',
  proposal: '提案截止',
  revision: '修改稿截止',
  decision: '结果通知',
  publication: '预计出版',
  event: '会议日期',
  registration: '报名 / 缴费',
};
