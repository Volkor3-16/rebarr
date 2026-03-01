/// All the possible tasks that need to run on schedule/trigger
pub enum TaskType {
    ScanLibrary,
    RefreshAniList,
    CheckNewChapter,
    DownloadChapter,
    Backup,
}