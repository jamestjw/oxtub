use std::cmp::Ordering;

use crate::{
    buffer::bpm::BufferPoolManager,
    catalog::schema::Schema,
    common::types::PageId,
    query::{
        executor::{
            engine::ExecutorRow,
            error::ExecutionError,
            executor::{Executor, ExecutorContext},
        },
        planner::plan::{PlannedOrderBy, SortPlan},
    },
    storage::{page::intermediate_result_page::IntermediateResultPage, table::tuple::Tuple},
    types::value::Value,
};

/// The sort key defines the list of values that sorting is based on.
pub type SortKey = Vec<Value>;

/// A sort entry pairs the sort key with its source tuple.
pub type SortEntry = (SortKey, Tuple);

/// Compares sort keys and sort entries using the planned ORDER BY clauses.
pub struct TupleComparator {
    order_bys: Vec<PlannedOrderBy>,
}

impl TupleComparator {
    pub fn new(order_bys: Vec<PlannedOrderBy>) -> Self {
        Self { order_bys }
    }

    // TODO: dont think this is needed right?
    // pub fn compare_entries(&self, entry_a: &SortEntry, entry_b: &SortEntry) -> Ordering {
    //     todo!("compare sort entries")
    // }

    pub fn compare_keys(&self, key_a: &SortKey, key_b: &SortKey) -> Ordering {
        todo!("compare sort keys")
    }
}

pub struct MergeSortRun<'bpm> {
    bpm: &'bpm BufferPoolManager,
    page_ids: Vec<PageId>,
}

impl<'bpm> MergeSortRun<'bpm> {
    pub fn new(bpm: &'bpm BufferPoolManager, page_ids: Vec<PageId>) -> Self {
        Self { bpm, page_ids }
    }

    pub fn page_ids(&self) -> &[PageId] {
        &self.page_ids
    }

    pub fn into_iter(self) -> MergeSortRunIterator<'bpm> {
        MergeSortRunIterator::new(self.bpm, self.page_ids)
    }
}

pub struct MergeSortRunIterator<'bpm> {
    bpm: &'bpm BufferPoolManager,
    page_ids: Vec<PageId>,
    page_idx: usize,
    tuple_idx: usize,
}

impl<'bpm> MergeSortRunIterator<'bpm> {
    pub fn new(bpm: &'bpm BufferPoolManager, page_ids: Vec<PageId>) -> Self {
        Self {
            bpm,
            page_ids,
            page_idx: 0,
            tuple_idx: 0,
        }
    }
}

impl Iterator for MergeSortRunIterator<'_> {
    type Item = Result<Tuple, ExecutionError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.page_idx >= self.page_ids.len() {
            return None;
        }

        let page_guard = match self.bpm.read_page(self.page_ids[self.page_idx]) {
            Ok(page_guard) => page_guard,
            Err(err) => return Some(Err(err.into())),
        };
        let page = IntermediateResultPage::from_data(page_guard.page_data());

        if self.tuple_idx >= page.num_tuples() {
            self.tuple_idx = 0;
            self.page_idx += 1;
            return self.next();
        }

        let tuple = page.get_tuple(self.tuple_idx);
        self.tuple_idx += 1;

        Some(Ok(tuple))
    }
}

pub struct ExternalMergeSortExecutor<'ctx, 'catalog, 'bpm, 'plan> {
    exec_ctx: &'ctx ExecutorContext<'catalog, 'bpm>,
    plan: &'plan SortPlan,
    output_schema: &'plan Schema,
    child: Box<dyn Executor + 'plan>,
    comparator: TupleComparator,
    sorted_page_ids: Vec<PageId>,
    sorted_iterator: Option<MergeSortRunIterator<'bpm>>,
    dropped_final_run: bool,
}

impl<'ctx, 'catalog, 'bpm, 'plan> ExternalMergeSortExecutor<'ctx, 'catalog, 'bpm, 'plan> {
    pub fn new(
        exec_ctx: &'ctx ExecutorContext<'catalog, 'bpm>,
        plan: &'plan SortPlan,
        output_schema: &'plan Schema,
        child: Box<dyn Executor + 'plan>,
    ) -> Self {
        Self {
            exec_ctx,
            plan,
            output_schema,
            child,
            comparator: TupleComparator::new(plan.order_bys.clone()),
            sorted_page_ids: Vec::new(),
            sorted_iterator: None,
            dropped_final_run: false,
        }
    }
}

impl Executor for ExternalMergeSortExecutor<'_, '_, '_, '_> {
    fn init(&mut self) -> Result<(), ExecutionError> {
        self.child.init()?;

        let final_sorted_run: MergeSortRun<'_> = todo!("perform external merge sort");
        self.sorted_page_ids = final_sorted_run.page_ids().to_vec();
        self.sorted_iterator = Some(final_sorted_run.into_iter());

        Ok(())
    }

    fn next(&mut self, batch_size: usize) -> Result<Vec<ExecutorRow>, ExecutionError> {
        todo!("pull sorted tuples from sorted_iterator")
    }

    fn output_schema(&self) -> &Schema {
        self.output_schema
    }
}
